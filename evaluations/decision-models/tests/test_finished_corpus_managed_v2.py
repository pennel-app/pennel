import hashlib
import json
from pathlib import Path
import re
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished_corpus_managed_v1 as original
import finished_corpus_managed_v2 as corpus


def canonical_bytes(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'),
                      allow_nan=False).encode()


def calibration_payload(spec):
    fixtures = [fixture for fixture in spec['fixtures'] if fixture['split'] == 'calibration']
    ids = [fixture['id'] for fixture in fixtures]
    return {'fixtures': fixtures,
            'expected_answers': {identifier: spec['expected_answers'][identifier] for identifier in ids},
            'gold_items': {identifier: spec['gold_items'][identifier] for identifier in ids}}


class ManagedV2CorpusTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.spec = corpus.experiment()
        cls.original = original.experiment()
        cls.heldout = [fixture for fixture in cls.spec['fixtures'] if fixture['split'] == 'heldout']

    def test_exactly_nine_new_heldout_families_have_all_eight_length_variants(self):
        self.assertEqual(len(self.spec['fixtures']), 170)
        self.assertEqual(len({fixture['id'] for fixture in self.spec['fixtures']}), 170)
        self.assertEqual(len(self.heldout), 72)
        self.assertEqual(len(corpus.FAMILIES), 9)
        families = {}
        for fixture in self.heldout:
            self.assertRegex(fixture['id'], r'^nl-managed-v2-heldout-f0[0-8]-v[0-7]$')
            self.assertEqual(fixture['language'], 'nl')
            families.setdefault(fixture['family_id'], []).append(fixture)
        self.assertEqual(len(families), 9)
        for family in families.values():
            self.assertEqual(len(family), 8)
            self.assertEqual({len(fixture['utterances']) for fixture in family}, set(range(8, 16)))
        self.assertEqual(sum(len(fixture['utterances']) for fixture in self.heldout), 828)

    def test_calibration_fixtures_answers_and_gold_are_byte_value_identical(self):
        current = calibration_payload(self.spec)
        previous = calibration_payload(self.original)
        self.assertEqual(len(current['fixtures']), 96)
        self.assertEqual(canonical_bytes(current), canonical_bytes(previous))
        # These source-data snapshots prevent silent regeneration after V1 edits.
        digests = {
            'fixtures': 'c3ebc64d6c95e464d0b7507bb7b53e3cdb522ccb201fe3468c2c00b015c3fa42',
            'expected_answers': '4dfec23777e6ea9e657614b337a6478057d205f09345a2ee0443cd572d12120e',
            'gold_items': '4ebe70e6c94f65e0ceca6c9f32b4a66aeae34a4148086d347cec13b9115e823b',
        }
        for key, digest in digests.items():
            self.assertEqual(hashlib.sha256(canonical_bytes(current[key])).hexdigest(), digest)

    def test_reuse_metadata_names_only_original_calibration_sources(self):
        ids = [fixture['id'] for fixture in self.original['fixtures'] if fixture['split'] == 'calibration']
        self.assertEqual(self.spec['calibration_reuse'], {
            'mode': 'exact_source_fixture_reuse',
            'source_corpus_id': self.original['corpus_id'],
            'source_module': 'finished_corpus_managed_v1.py',
            'fixture_ids': ids,
            'fixture_count': 96,
        })
        provenance = self.spec['provenance']
        self.assertFalse(provenance['real_meeting_content'])
        self.assertFalse(provenance['natural_asr'])
        self.assertFalse(provenance['historical_quality_credit'])
        self.assertFalse(provenance['prior_link_extraction'])
        self.assertIn('no model predictions', provenance['source'])
        self.assertIn('held-out', provenance['wording'])

    def test_only_unchanged_development_panels_are_retained_from_seen_material(self):
        previous = {fixture['id']: fixture for fixture in self.original['fixtures']}
        development = [fixture for fixture in self.spec['fixtures'] if fixture['split'] == 'development']
        self.assertEqual(len(development), 2)
        for key, length in [('resource_fixture_id', 16), ('stress_fixture_id', 512)]:
            identifier = self.spec[key]
            self.assertEqual(identifier, self.original[key])
            fixture = next(fixture for fixture in development if fixture['id'] == identifier)
            self.assertEqual(len(fixture['utterances']), length)
            self.assertEqual(canonical_bytes(fixture), canonical_bytes(previous[identifier]))
            for field in ['expected_answers', 'gold_items']:
                self.assertEqual(canonical_bytes(self.spec[field][identifier]),
                                 canonical_bytes(self.original[field][identifier]))
        self.assertFalse(any(fixture['id'].startswith('nl-managed-v1-heldout-')
                             for fixture in self.spec['fixtures']))

    def test_new_families_and_texts_are_disjoint_from_every_v1_split(self):
        old_families = {fixture['family_id'] for fixture in self.original['fixtures']}
        old_topics = {family[0] for split in original.FAMILIES.values() for family in split}
        old_texts = {row['text'] for fixture in self.original['fixtures'] for row in fixture['utterances']}
        old_wordings = {wording for split in original.WORDINGS.values() for wording in split}
        new_texts = [row['text'] for fixture in self.heldout for row in fixture['utterances']]
        self.assertTrue({fixture['family_id'] for fixture in self.heldout}.isdisjoint(old_families))
        self.assertTrue({family[0] for family in corpus.FAMILIES}.isdisjoint(old_topics))
        self.assertTrue(set(new_texts).isdisjoint(old_texts))
        self.assertEqual(len(set(new_texts)), len(new_texts))
        self.assertEqual(len(corpus.WORDINGS), 16)
        self.assertEqual(len(set(corpus.WORDINGS)), 16)
        self.assertTrue(set(corpus.WORDINGS).isdisjoint(old_wordings))
        for fixture in self.spec['fixtures']:
            if fixture['family_id'] in {item['family_id'] for item in self.heldout}:
                self.assertEqual(fixture['split'], 'heldout')

    def test_every_family_covers_all_sixteen_existing_semantics(self):
        self.assertEqual(corpus.CASE_STATES, original.CASE_STATES)
        self.assertEqual(corpus.CASE_SUPPORT, original.CASE_SUPPORT)
        for family in range(9):
            seen = set()
            for variant in range(8):
                fixture, expected, _ = corpus.fixture(family, variant)
                self.assertEqual(len(fixture['utterances']), 8 + variant)
                for index, labels in enumerate(expected):
                    case = corpus.case_index(family, variant, index)
                    seen.add(case)
                    for kind in corpus.KINDS:
                        self.assertEqual(labels[kind], corpus.CASE_STATES[case].get(kind, 'none'))
                        self.assertEqual(labels[kind + '_support'], corpus.CASE_SUPPORT[case][kind])
            self.assertEqual(seen, set(range(16)))
        self.assertEqual(corpus.CASE_STATES[6], {})  # Quoted rhetorical non-event.
        self.assertEqual(corpus.CASE_STATES[13], {})  # Previously unknown fact now resolved.
        self.assertEqual(corpus.CASE_STATES[14], {'action': 'unknown'})  # Firm and proposed tasks coexist.
        self.assertEqual(corpus.CASE_STATES[7], {'action': 'retracted', 'changed_commitment': 'retracted'})
        self.assertEqual(corpus.CASE_STATES[3], {'decision': 'changed', 'changed_commitment': 'changed'})

    def test_every_row_and_label_matches_the_existing_ontology(self):
        keys = set(corpus.KINDS) | {kind + '_support' for kind in corpus.KINDS}
        source_keys = {'start_ms', 'end_ms', 'speaker', 'text', 'failed', 'voice'}
        states, support = set(), set()
        ids = {fixture['id'] for fixture in self.spec['fixtures']}
        self.assertEqual(set(self.spec['expected_answers']), ids)
        self.assertEqual(set(self.spec['gold_items']), ids)
        for fixture in self.spec['fixtures']:
            expected = self.spec['expected_answers'][fixture['id']]
            self.assertEqual(len(expected), len(fixture['utterances']))
            for index, (source, labels) in enumerate(zip(fixture['utterances'], expected)):
                self.assertEqual(set(source), source_keys)
                self.assertEqual(source['start_ms'], index * 5000)
                self.assertEqual(source['end_ms'], index * 5000 + 4000)
                self.assertEqual(source['speaker'], 'others')
                self.assertIs(source['failed'], False)
                self.assertIsNone(source['voice'])
                self.assertEqual(set(labels), keys)
                for kind in corpus.KINDS:
                    self.assertIn(labels[kind], corpus.STATES)
                    self.assertIn(labels[kind + '_support'], corpus.SUPPORT)
                    states.add(labels[kind])
                    support.add(labels[kind + '_support'])
        self.assertEqual(states, set(original.STATES))
        self.assertEqual(support, {'supported', 'absent'})

    def test_complete_literal_gold_has_valid_source_indices_and_null_extraction(self):
        for fixture in self.spec['fixtures']:
            expected = self.spec['expected_answers'][fixture['id']]
            actual = set()
            for item in self.spec['gold_items'][fixture['id']]:
                self.assertEqual(set(item), {'window_index', 'kind', 'commitment', 'text', 'owner', 'due'})
                index = item['window_index']
                self.assertIs(type(index), int)
                self.assertGreaterEqual(index, 0)
                self.assertLess(index, len(fixture['utterances']))
                self.assertIn(item['kind'], corpus.KINDS)
                self.assertEqual(item['text'].encode(), fixture['utterances'][index]['text'].encode())
                self.assertEqual(item['commitment'], expected[index][item['kind']])
                self.assertEqual(expected[index][item['kind'] + '_support'], 'supported')
                self.assertNotIn(item['commitment'], ['none', 'negated'])
                self.assertIsNone(item['owner'])
                self.assertIsNone(item['due'])
                key = (index, item['kind'], item['commitment'])
                self.assertNotIn(key, actual)
                actual.add(key)
            required = {(index, kind, labels[kind]) for index, labels in enumerate(expected)
                        for kind in corpus.KINDS
                        if labels[kind + '_support'] == 'supported' and labels[kind] not in ('none', 'negated')}
            self.assertEqual(actual, required)

    def test_authored_content_uses_fictional_references_without_personal_data(self):
        # Content is generated only from the local authored banks and mineral labels.
        self.assertEqual(corpus.REFERENCES,
                         ('Agaat', 'Basalt', 'Calciet', 'Dolomiet', 'Fluoriet', 'Gneis', 'Kwarts', 'Leisteen', 'Marmer'))
        private_patterns = [r'\b[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}\b', r'https?://', r'/Users/',
                            r'\b\d{6,}\b', r'\b(?:IBAN|BSN|rijksregisternummer|telefoonnummer)\b']
        for fixture in self.spec['fixtures']:
            for row in fixture['utterances']:
                self.assertIsNone(row['voice'])
                for pattern in private_patterns:
                    self.assertIsNone(re.search(pattern, row['text'], re.IGNORECASE))
        for fixture in self.heldout:
            for row in fixture['utterances']:
                self.assertRegex(row['text'], r' / (?:' + '|'.join(corpus.REFERENCES) + r')-[1-8]-\d{3}: ')
                self.assertLessEqual(len(row['text'].split()), 65)

    def test_experiment_is_reproducible_and_configuration_cannot_mutate_source_values(self):
        self.assertEqual(self.spec['corpus_id'], 'authored-managed-decider-meetings-v2')
        self.assertEqual(self.spec, corpus.experiment())
        self.assertNotIn('configuration', self.spec)
        self.assertNotIn('expected_actual', self.spec)
        configuration = {'protocol': 'pennel_cloud', 'nested': {'model': 'review-test'}}
        identity = {'model': 'separately verified identity'}
        routed = corpus.experiment(configuration, identity)
        configuration['nested']['model'] = 'changed'
        identity['model'] = 'changed'
        self.assertEqual(routed['configuration']['nested']['model'], 'review-test')
        self.assertEqual(routed['expected_actual']['model'], 'separately verified identity')
        routed['fixtures'][0]['utterances'][0]['text'] = 'changed'
        routed['calibration_reuse']['fixture_ids'].clear()
        self.assertEqual(self.spec, corpus.experiment())
        self.assertEqual(self.original, original.experiment())
        for arguments in [(configuration, None), (None, identity)]:
            with self.assertRaises(ValueError):
                corpus.experiment(*arguments)

    def test_fixture_bounds_are_fixed_and_reject_boolean_indices(self):
        for family, variant in [(-1, 0), (9, 0), (0, -1), (0, 8), (True, 0), (0, False), ('0', 0)]:
            with self.assertRaises(ValueError):
                corpus.fixture(family, variant)


if __name__ == '__main__':
    unittest.main()
