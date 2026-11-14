import copy
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished
import finished_calibration_reuse as reuse
import finished_qualification as qualification
import finished_support_v1 as support
import finished_cloud_experiment as cloud
import calibration
import metrics
from test_finished_qualification_v3 import SupportBridge, experiment, predictions
from test_finished_cloud_experiment import Bridge as CloudBridge, Client as CloudClient, exported as cloud_export
from runners import SpendLedger


def exported():
    return {'schema_version': 3, 'protocol_id': 'production-finished-meeting-v3',
        'runtime_policy': qualification.RUNTIME_POLICY_V3, 'production_admission': False,
        'qualification': False, 'fixture': {'id': 'owned-cal', 'split': 'calibration',
            'family_id': 'owned-cal-family', 'utterances': [{'text': 'Invented calibration.'}]},
        'configuration': {'model': 'owned-model'}, 'expected_actual': {'model': 'owned-actual'},
        'snapshot': {'revision': 1}, 'windows': [{'request': {'questions': ['owned']},
            'prompt_context_sha256': 'a'*64, 'sources': [{'quote': 'Invented only.'}]}],
        'batches': [{'wire_body': '{"owned":true}', 'answer_map': ['owned']}],
        'qualification_request': {'prompt_sha256': 'b'*64, 'grouping': 'owned',
            'label_order_sha256': 'c'*64, 'schema_sha256': 'd'*64,
            'preprocessing_sha256': 'e'*64}, 'source_sha256': {'owned': 'f'*64},
        'freeze_sha256': '0'*64}


class CalibrationReuseTests(unittest.TestCase):
    def gated(self):
        value = exported()
        value.update(runtime_policy=support.RUNTIME_POLICY, support_calibration={'owned': True},
            source_sha256={'owned': '1'*64}, freeze_sha256='2'*64)
        value['qualification_request'].update(schema_sha256='3'*64, preprocessing_sha256='4'*64)
        return value

    def test_only_explicit_postprocessing_metadata_can_change(self):
        self.assertEqual(reuse.input_projection(exported()), reuse.input_projection(self.gated()))
        for field in ('configuration', 'expected_actual', 'snapshot', 'windows', 'batches'):
            value = self.gated(); value[field] = {'changed': True}
            self.assertNotEqual(reuse.input_projection(exported()), reuse.input_projection(value))
        for field in ('prompt_sha256', 'label_order_sha256', 'grouping'):
            value = self.gated(); value['qualification_request'][field] = 'changed'
            self.assertNotEqual(reuse.input_projection(exported()), reuse.input_projection(value))

    def test_unknown_export_fields_are_not_silently_ignored(self):
        value = self.gated(); value['new_input_policy'] = 'changed'
        self.assertNotEqual(reuse.input_projection(exported()), reuse.input_projection(value))

    def pair(self):
        old, current = exported(), self.gated()
        held = copy.deepcopy(current)
        held['fixture'].update(id='fresh-held', split='heldout', family_id='fresh-family',
            utterances=[{'text': 'Entirely new heldout source.'}])
        source = {'production': {'exports': [old], 'expected_answers': {'owned-cal': [{'action': 'none'}]}},
            'gold_items': {'owned-cal': []}}
        fresh = {'production': {'exports': [current, held], 'expected_answers': {'owned-cal': [{'action': 'none'}]}},
            'gold_items': {'owned-cal': []}}
        return source, fresh

    def test_reuse_compares_exact_inputs_judgments_and_contexts(self):
        source, fresh = self.pair()
        self.assertEqual(len(reuse.equivalence(fresh, source)), 64)
        for field in ('wire', 'context', 'answers', 'gold'):
            changed = copy.deepcopy(fresh)
            if field == 'wire': changed['production']['exports'][0]['batches'][0]['wire_body'] = 'changed'
            if field == 'context': changed['production']['exports'][0]['windows'][0]['prompt_context_sha256'] = 'changed'
            if field == 'answers': changed['production']['expected_answers']['owned-cal'][0]['action'] = 'confirmed'
            if field == 'gold': changed['gold_items']['owned-cal'] = [{'invented': True}]
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'calibration_model_input_or_judgment_changed'):
                reuse.equivalence(changed, source)

    def test_old_cases_families_or_sources_cannot_become_fresh_heldout(self):
        source, fresh = self.pair()
        for field in ('id', 'family_id', 'utterances'):
            changed = copy.deepcopy(fresh)
            changed['production']['exports'][1]['fixture'][field] = source['production']['exports'][0]['fixture'][field]
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'fresh_heldout_after_runtime_change_required'):
                reuse.equivalence(changed, source)

    def test_asset_retains_original_fit_and_manifest_without_claiming_approval(self):
        value, sha, artifact = reuse.asset()
        self.assertEqual(finished.digest(artifact), reuse.ARTIFACT_SHA256)
        self.assertEqual(artifact['run_manifest_hash'], reuse.SOURCE_MANIFEST)
        self.assertEqual(value['provenance']['fitted_lock_commit'], reuse.FITTED_LOCK_COMMIT)
        self.assertFalse(value['production_admission']); self.assertFalse(value['qualification'])
        self.assertEqual(len(sha), 64)
        value['provenance']['source_manifest_sha256'] = '0'*64
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'changed-asset.json'
            path.write_text(json.dumps(value))
            with patch.object(reuse, 'ASSET', path), self.assertRaisesRegex(ValueError, 'unlinked_runtime_support_calibration'):
                reuse.asset()

    def test_protocol_preserves_every_quality_resource_and_budget_gate(self):
        root = Path(qualification.__file__).parent
        original = json.loads((root/'protocol-finished-qualification-cloud-v1.json').read_text())
        gated = json.loads(support.QUALIFICATION_PROTOCOL.read_text())
        self.assertEqual(gated['acceptance'], original['acceptance'])
        self.assertEqual(gated['bounds'], original['bounds'])
        self.assertEqual(gated['runtime_policy'], support.RUNTIME_POLICY)
        self.assertTrue(gated['calibration_reuse']['preserve_original_run_manifest_hash'])
        self.assertTrue(gated['calibration_reuse']['exact_model_input_equivalence_required'])
        self.assertFalse(gated['calibration_reuse']['counts_as_new_inference'])
        self.assertEqual(gated['fresh_evidence']['heldout_fixtures'], 72)
        self.assertEqual(gated['fresh_evidence']['cold_samples'], 5)
        self.assertEqual(gated['fresh_evidence']['warm_samples'], 30)

    def test_reused_run_keeps_its_original_linkage_and_cannot_bind_fresh_heldout(self):
        old_export = exported(); new_export = self.gated()
        old_export['batches'] = new_export['batches'] = [{'wire_body': '{}'}]
        old = {'freeze_sha256': 'a'*64}
        new = {'freeze_sha256': 'b'*64}
        run = {'freeze_sha256': old['freeze_sha256'], 'export_sha256': old_export['freeze_sha256'],
            'evidence_class': 'recorded_inference', 'responses': [], 'request_sha256s': []}
        kept = copy.deepcopy(run)
        bridge = lambda request: {'complete': False}
        qualification._run(new, new_export, run, bridge, original=(old, old_export))
        self.assertEqual(run, kept)
        with self.assertRaisesRegex(ValueError, 'unlinked_prediction_or_changed_request'):
            qualification._run(new, new_export, run, bridge)
        new_export['fixture']['split'] = 'heldout'
        with self.assertRaisesRegex(ValueError, 'calibration_reuse_only'):
            qualification._run(new, new_export, run, bridge, original=(old, old_export))

    def test_changed_inputs_refuse_replay_even_with_original_recorded_hashes(self):
        old, new = exported(), self.gated()
        old['batches'] = [{'wire_body': '{}'}]
        run = {'freeze_sha256': 'a'*64, 'export_sha256': old['freeze_sha256'],
            'evidence_class': 'recorded_inference', 'responses': [], 'request_sha256s': []}
        with self.assertRaisesRegex(ValueError, 'calibration_model_input_or_judgment_changed'):
            qualification._run({'freeze_sha256':'b'*64}, new, run, lambda _: {},
                original=({'freeze_sha256':'a'*64}, old))

    def test_new_policy_refuses_another_calibration_phase(self):
        frozen = {'protocol_id':'production-finished-qualification-cloud-support-v1', 'stage':'calibration'}
        with patch.object(qualification, 'verify_freeze'):
            with self.assertRaisesRegex(ValueError, 'calibration_reused_without_new_inference'):
                qualification.requests(frozen, 'calibration', None)
            with self.assertRaisesRegex(ValueError, 'explicit_original_calibration_reuse_required'):
                qualification.calibrate(frozen, [], None)

    def test_raw_policy_and_its_original_protocol_remain_separate(self):
        raw = {'schema_version':3, 'protocol_id':'production-finished-qualification-cloud-v1'}
        gated = {'schema_version':3, 'protocol_id':'production-finished-qualification-cloud-support-v1'}
        self.assertEqual(qualification._runtime_policy(raw), qualification.RUNTIME_POLICY_V3)
        self.assertEqual(qualification._runtime_policy(gated), support.RUNTIME_POLICY)
        self.assertEqual(qualification._protocol(raw), qualification.PROTOCOL_CLOUD)
        self.assertEqual(qualification._protocol(gated), support.QUALIFICATION_PROTOCOL)
        self.assertIsNot(qualification._contract(raw), qualification._contract(gated))

    def test_reuse_audit_does_not_relabel_or_change_the_new_base_manifest(self):
        base = qualification._seal({'stage':'calibration', 'production_admission':False, 'owned':'base'})
        ready = copy.deepcopy(base)
        ready.update(stage='heldout_ready', base_freeze_sha256=base['freeze_sha256'], calibration={'run_manifest_hash':reuse.SOURCE_MANIFEST},
            calibration_runs=[{'freeze_sha256':reuse.SOURCE_MANIFEST}], calibration_runs_sha256='a'*64,
            calibration_reuse={'counts_as_new_inference':False},
            calibration_replay_verification={'max_probability_absolute_delta':1e-16})
        qualification._seal(ready)
        self.assertEqual(qualification._base(ready), base)
        self.assertEqual(ready['calibration']['run_manifest_hash'], reuse.SOURCE_MANIFEST)
        self.assertEqual(ready['calibration_runs'][0]['freeze_sha256'], reuse.SOURCE_MANIFEST)

    def test_new_runtime_cannot_use_old_seen_authored_corpus(self):
        canonical = cloud.managed_corpus.experiment()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'seen-raw-corpus.json'; path.write_text(json.dumps(canonical))
            with self.assertRaisesRegex(ValueError, 'canonical_fresh_managed_corpus_required'):
                cloud.authored_spec(path, {'protocol_id':'production-finished-qualification-cloud-support-v1'})

    def test_typed_replay_uses_retained_actual_engine_result_without_rewriting_old_run(self):
        result = {'answers':[], 'identity':{'request_id':'owned'}, 'usage':None}
        response = {'batch_index':0, 'http_status':200, 'body':'{"owned":true}', 'request_id':'owned'}
        run = {'responses':[response], 'requests':[{'batch_index':0, 'outcome':{'result':result}}]}
        kept = copy.deepcopy(run)
        replay = reuse.typed_responses(run, reused=True)
        self.assertEqual(replay, [{**response, 'production_result':result}])
        self.assertEqual(run, kept)
        with self.assertRaisesRegex(ValueError, 'recorded_engine_result_required'):
            reuse.typed_responses(run)
        new = copy.deepcopy(run); new['responses'][0]['production_result'] = copy.deepcopy(result)
        self.assertEqual(reuse.typed_responses(new), replay)
        new['responses'][0]['production_result']['identity']['request_id'] = 'fabricated'
        with self.assertRaisesRegex(ValueError, 'recorded_engine_result_required'):
            reuse.typed_responses(new)

    def test_logical_replay_must_preserve_every_recorded_engine_value_exactly(self):
        value = {'windows':[{'batch_index':0}], 'batches':[{'index':0, 'window_indices':[0],
            'answer_map':[{'window_index':0, 'batch_id':'w0_action', 'original_id':'action'}]}]}
        physical = {'answers':[{'id':'w0_action', 'choice':'confirmed', 'probabilities':[.7, .3]}],
            'identity':{'request_id':'owned'}, 'usage':{'input_tokens':10}}
        logical = copy.deepcopy(physical); logical['answers'][0]['id'] = 'action'
        responses = [{'batch_index':0, 'production_result':physical}]
        replay = {'windows':[{'window_index':0, 'status':'ok', 'result':logical}]}
        reuse.verify_engine_replay(value, responses, replay)
        replay['windows'][0]['result']['answers'][0]['probabilities'][0] = .7000000000000001
        with self.assertRaisesRegex(ValueError, 'actual_engine_replay_changed'):
            reuse.verify_engine_replay(value, responses, replay)

    def test_serialization_delta_is_audited_without_refitting_or_changing_classes(self):
        original = [{'id':'owned', 'prediction':'confirmed', 'run_manifest_hash':reuse.SOURCE_MANIFEST,
            'probabilities':{'confirmed':.7, 'none':.3}}]
        current = copy.deepcopy(original); current[0]['probabilities']['confirmed'] = .7000000000000001
        kept = copy.deepcopy(original)
        delta = reuse.serialization_delta(original, current)
        self.assertEqual(delta['changed_probability_values'], 1)
        self.assertLess(delta['max_probability_absolute_delta'], 1e-12)
        self.assertEqual(original, kept); self.assertFalse(delta['counts_as_new_inference'])
        current[0]['prediction'] = 'none'
        with self.assertRaisesRegex(ValueError, 'calibration_raw_engine_context_changed'):
            reuse.serialization_delta(original, current)
        current = copy.deepcopy(original); current[0]['probabilities']['confirmed'] = .8
        with self.assertRaisesRegex(ValueError, 'calibration_serialization_delta_not_rounding'):
            reuse.serialization_delta(original, current)

    def test_batched_replay_keeps_usage_physical_without_changing_recorded_result(self):
        value = {'windows':[{'batch_index':0}], 'batches':[{'index':0, 'window_indices':[0, 1],
            'answer_map':[{'window_index':0, 'batch_id':'w0_action', 'original_id':'action'}]}]}
        physical = {'answers':[{'id':'w0_action', 'choice':'confirmed'}],
            'identity':{'request_id':'upstream-gen-id'}, 'usage':{'input_tokens':10, 'output_tokens':1, 'billed_cost_usd':.001}}
        logical = copy.deepcopy(physical); logical['answers'][0]['id'] = 'action'
        logical['usage'] = {'input_tokens':None, 'output_tokens':None, 'billed_cost_usd':None}
        responses = [{'batch_index':0, 'request_id':'wire-uuid', 'production_result':physical}]
        kept = copy.deepcopy(physical)
        reuse.verify_engine_replay(value, responses, {'windows':[{'window_index':0, 'status':'ok', 'result':logical}]})
        self.assertEqual(physical, kept)
        logical['usage'] = copy.deepcopy(physical['usage'])
        with self.assertRaisesRegex(ValueError, 'actual_engine_replay_changed'):
            reuse.verify_engine_replay(value, responses, {'windows':[{'window_index':0, 'status':'ok', 'result':logical}]})

    def test_new_cloud_recorder_copies_the_actual_outcome_and_retains_raw_capture(self):
        owned_result = {'answers':[], 'identity':{'request_id':'owned-engine-id'}, 'usage':None}
        class TypedClient(CloudClient):
            def call(self, value, deadline):
                response = super().call(value, deadline)
                response['outcome']['result'] = copy.deepcopy(owned_result)
                return response
        value = cloud_export(); value['runtime_policy'] = support.RUNTIME_POLICY
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'run.json'
            with patch.object(reuse, 'metadata'), patch.object(finished, '_verify_export'):
                run = cloud.run_job(value, 'a'*64, CloudBridge(), b'owned-fake-access-only-session',
                    SpendLedger(), path, execute=True, client_factory=TypedClient)
            self.assertEqual(run['status'], 'ok')
            self.assertEqual(run['responses'][0]['production_result'], owned_result)
            self.assertEqual(run['responses'][0]['production_result'], run['requests'][0]['outcome']['result'])
            self.assertEqual(run['responses'][0]['body'], run['requests'][0]['raw_response'])
            self.assertEqual(run['responses'][0]['http_status'], 200)
            self.assertEqual(run['responses'][0]['request_id'], run['requests'][0]['request_id'])
            self.assertNotIn('owned-fake-access-only-session', path.read_text())


class RuntimeGateEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.bridge = SupportBridge()
        cls.base = experiment(cls.bridge)
        cls.ready = qualification.calibrate(cls.base, predictions(cls.base, 'calibration'), cls.bridge)
        cls.export = next(x for x in cls.base['production']['exports'] if x['fixture']['split'] == 'calibration')
        run = predictions(cls.base, 'calibration')[0]
        cls.replay = cls.bridge({'op':'finished_meeting_replay', 'export':cls.export, 'responses':run['responses']})['windows'][0]

    def case(self, reason='accepted'):
        replay = copy.deepcopy(self.replay)
        decisions = []
        answers = {x['id']:x for x in replay['result']['answers']}
        window = self.export['windows'][0]
        for question in window['request']['questions']:
            key = question['id']
            if not key.endswith('_support'): continue
            answer = answers[key]
            if key == 'action_support':
                if reason == 'model_abstained': answer['abstained'] = True
                if reason == 'raw_unclear': answer['choice'] = 'unclear'
                if reason == 'choice_not_argmax': answer['choice'] = 'supported'
                if reason == 'below_frozen_threshold':
                    answer['probabilities'] = [{'option_id':label, 'probability':probability}
                        for label, probability in (('absent', .5), ('supported', .3), ('unclear', .2))]
            row = {'task':'events', 'subtask':key, 'language':'nl', 'runner_id':'production-finished',
                'label_order':[x['id'] for x in question['options']],
                'prompt_hash':finished.digest({'question':question, 'physical_prompt_context_sha256':window['prompt_context_sha256']}),
                'grouping':self.export['qualification_request']['grouping'],
                'identity':{'transport':self.export['configuration']['protocol'], **self.export['expected_actual']}}
            context = calibration.prediction_context(row); context_hash = metrics._context_hash(context)
            fit = self.ready['calibration']['fits'][context_hash]['fit']
            distribution = metrics.apply_calibration({x['option_id']:x['probability'] for x in answer['probabilities']}, fit, context)
            choice = max(sorted(distribution), key=distribution.get)
            why = reason if key == 'action_support' else 'accepted'
            decisions.append({'question_id':key, 'context_sha256':context_hash, 'raw_choice':answer['choice'],
                'calibrated_choice':choice, 'calibrated_probability':distribution[choice],
                'threshold':fit['threshold']['threshold'], 'accepted':why == 'accepted', 'reason':why})
        replay['support_gate'] = decisions
        replay['assessed_kinds'] = [x for x in replay['assessed_kinds'] if reason == 'accepted' or x != 'action']
        return replay

    def test_each_runtime_reason_keeps_raw_result_and_has_verified_gate_evidence(self):
        for reason in ('accepted', 'model_abstained', 'raw_unclear', 'choice_not_argmax', 'below_frozen_threshold'):
            replay = self.case(reason); kept = copy.deepcopy(replay['result'])
            with self.subTest(reason=reason):
                reuse.verify_gate_window(self.export, self.export['windows'][0], replay, self.ready['calibration'])
                self.assertEqual(kept, replay['result'])

    def test_gate_metadata_or_rejected_scope_cannot_be_fabricated(self):
        for field in ('missing', 'accepted', 'context', 'probability', 'scope'):
            replay = self.case('below_frozen_threshold')
            if field == 'missing': replay['support_gate'].pop()
            if field == 'accepted': replay['support_gate'][0]['accepted'] = True
            if field == 'context': replay['support_gate'][0]['context_sha256'] = '0'*64
            if field == 'probability': replay['support_gate'][0]['calibrated_probability'] = 1
            if field == 'scope': replay['assessed_kinds'].append('action')
            with self.subTest(field=field), self.assertRaises(ValueError):
                reuse.verify_gate_window(self.export, self.export['windows'][0], replay, self.ready['calibration'])


if __name__ == '__main__':
    unittest.main()
