import copy
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished_corpus_v2 as corpus


class AuthoredCorpusTests(unittest.TestCase):
    def setUp(self): self.spec = corpus.experiment()

    def test_families_and_verbatim_text_do_not_cross_splits(self):
        owners, texts = {}, {}
        for fixture in self.spec['fixtures']:
            split = fixture['split']
            self.assertEqual(owners.setdefault(fixture['family_id'], split), split)
            for row in fixture['utterances']:
                self.assertEqual(texts.setdefault(row['text'], split), split)
        self.assertFalse(self.spec['provenance']['natural_asr'])
        self.assertFalse(self.spec['provenance']['real_meeting_content'])

    def test_all_full_and_ragged_prompt_contexts_have_calibration_support(self):
        coverage = corpus.context_coverage(self.spec)
        self.assertEqual(len(coverage), sum(range(1, 9)) * 4)
        for row in coverage:
            self.assertGreaterEqual(row['observations'], 10, row)
            self.assertGreaterEqual(row['classes'], 2, row)
            self.assertGreaterEqual(row['families'], 2, row)

    def test_authored_gold_is_literal_source_and_never_invents_supported_details(self):
        for fixture in self.spec['fixtures']:
            self.assertEqual(len(self.spec['expected_answers'][fixture['id']]), len(fixture['utterances']))
            for judgment in self.spec['gold_items'][fixture['id']]:
                text = fixture['utterances'][judgment['window_index']]['text']
                self.assertEqual(judgment['text'], text)
                self.assertIsNone(judgment['owner'])
                self.assertIsNone(judgment['due'])
                self.assertNotIn(judgment['commitment'], ('none', 'negated'))

    def test_long_stress_and_fixed_resource_source_are_development_only(self):
        by_id = {item['id']: item for item in self.spec['fixtures']}
        panel = by_id[self.spec['resource_fixture_id']]
        stress = by_id[self.spec['stress_fixture_id']]
        self.assertEqual(panel['split'], 'development')
        self.assertEqual(len(panel['utterances']), 16)
        self.assertEqual(stress['split'], 'development')
        self.assertEqual(len(stress['utterances']), 512)
        self.assertEqual(self.spec, corpus.experiment())


if __name__ == '__main__': unittest.main()
