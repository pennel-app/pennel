import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished_corpus_v2 as old
import finished_corpus_v3 as corpus


class SupportedCorpusTests(unittest.TestCase):
    def setUp(self): self.spec = corpus.experiment()

    def test_fresh_source_text_and_families_stay_disjoint(self):
        prior = {row['text'] for attempt in ('A', 'B') for fixture in old.experiment(attempt)['fixtures']
                 for row in fixture['utterances']}
        owners, texts = {}, {}
        for fixture in self.spec['fixtures']:
            split = fixture['split']
            self.assertEqual(owners.setdefault(fixture['family_id'], split), split)
            for row in fixture['utterances']:
                if fixture['id'] != self.spec['seen_regression_fixture_id']:
                    self.assertNotIn(row['text'], prior)
                self.assertEqual(texts.setdefault(row['text'], split), split)
        self.assertFalse(self.spec['provenance']['real_meeting_content'])
        self.assertFalse(self.spec['provenance']['natural_asr'])

    def test_every_window_has_independent_state_and_support_gold(self):
        keys = set(corpus.KINDS) | {kind+'_support' for kind in corpus.KINDS}
        for fixture in self.spec['fixtures']:
            expected = self.spec['expected_answers'][fixture['id']]
            self.assertEqual(len(expected), len(fixture['utterances']))
            for row in expected:
                self.assertEqual(set(row), keys)
                for kind in corpus.KINDS:
                    self.assertIn(row[kind], corpus.STATES)
                    self.assertIn(row[kind+'_support'], corpus.SUPPORT)
            for item in self.spec['gold_items'][fixture['id']]:
                self.assertEqual(item['text'], fixture['utterances'][item['window_index']]['text'])
                self.assertEqual(expected[item['window_index']][item['kind']+'_support'], 'supported')
                self.assertIsNone(item['owner']);self.assertIsNone(item['due'])

    def test_questions_choices_and_mixed_commitments_keep_distinct_support(self):
        self.assertEqual(corpus.CASE_SUPPORT[2]['action'], 'absent')
        self.assertEqual(corpus.CASE_SUPPORT[3]['action'], 'absent')
        self.assertEqual(corpus.CASE_SUPPORT[10]['open_question'], 'absent')
        self.assertEqual(corpus.CASE_SUPPORT[14]['action'], 'supported')
        self.assertEqual(corpus.CASE_STATES[14]['action'], 'unknown')
        self.assertEqual(corpus.CASE_SUPPORT[12]['action'], 'supported')
        self.assertEqual(corpus.CASE_SUPPORT[12]['decision'], 'supported')

    def test_resource_and_stress_inputs_are_development_only(self):
        by_id = {fixture['id']:fixture for fixture in self.spec['fixtures']}
        for key, length in (('resource_fixture_id',16), ('stress_fixture_id',512)):
            fixture = by_id[self.spec[key]]
            self.assertEqual(fixture['split'],'development')
            self.assertEqual(len(fixture['utterances']),length)
        self.assertEqual(self.spec,corpus.experiment())

    def test_seen_b_regression_is_explicit_development_only_without_quality_credit(self):
        fixture=next(value for value in self.spec['fixtures'] if value['id']==self.spec['seen_regression_fixture_id'])
        self.assertEqual(fixture['split'],'development')
        self.assertEqual(len(fixture['utterances']),207)
        source=__import__('json').loads(Path(corpus.__file__).with_name('fixtures').joinpath('seen-b-semantic-regression-v3.json').read_text())
        self.assertFalse(source['quality_credit'])
        self.assertEqual(len(source['lineage']),69)
        self.assertEqual(source['evidence_class'],'seen_development_regression')


if __name__ == '__main__': unittest.main()
