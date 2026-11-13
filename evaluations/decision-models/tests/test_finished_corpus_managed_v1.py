from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished_corpus_managed_v1 as corpus
import finished_corpus_v2 as historical
import finished_corpus_v3 as previous


class ManagedCorpusTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.spec = corpus.experiment()

    def test_split_families_counts_and_all_length_variants_are_disjoint(self):
        fixtures = self.spec['fixtures']
        self.assertEqual(len(fixtures), 290)
        self.assertEqual(len({f['id'] for f in fixtures}), len(fixtures))
        families, texts = {}, {}
        for f in fixtures:
            self.assertTrue(f['id'].startswith('nl-managed-v1-'))
            self.assertEqual(families.setdefault(f['family_id'], f['split']), f['split'])
            self.assertEqual(f['language'], 'nl')
            for row in f['utterances']:
                self.assertEqual(texts.setdefault(row['text'], f['split']), f['split'])
        for split in ['calibration', 'heldout']:
            group = [f for f in fixtures if f['split'] == split]
            self.assertEqual(len(group), 96)
            self.assertEqual(len({f['family_id'] for f in group}), 12)
            self.assertEqual({len(f['utterances']) for f in group}, set(range(8, 16)))
        for left in corpus.FAMILIES:
            for right in corpus.FAMILIES:
                if left != right:
                    self.assertTrue({x[0] for x in corpus.FAMILIES[left]}.isdisjoint(x[0] for x in corpus.FAMILIES[right]))

    def test_no_historical_source_or_seen_regression_gets_fresh_credit(self):
        old = {row['text'] for old_spec in [historical.experiment('A'), historical.experiment('B'), previous.experiment()]
               for f in old_spec['fixtures'] for row in f['utterances']}
        fresh = {row['text'] for f in self.spec['fixtures'] for row in f['utterances']}
        self.assertTrue(fresh.isdisjoint(old))
        self.assertNotIn('seen_regression_fixture_id', self.spec)
        self.assertFalse(self.spec['provenance']['historical_quality_credit'])
        self.assertFalse(self.spec['provenance']['real_meeting_content'])
        self.assertFalse(self.spec['provenance']['natural_asr'])
        self.assertEqual(set(corpus.WORDINGS), set(corpus.FAMILIES))
        self.assertTrue(set(corpus.WORDINGS['calibration']).isdisjoint(corpus.WORDINGS['heldout']))

    def test_every_authored_window_has_complete_independent_judgments(self):
        keys = set(corpus.KINDS) | {kind + '_support' for kind in corpus.KINDS}
        all_states, all_support = set(), set()
        for f in self.spec['fixtures']:
            expected = self.spec['expected_answers'][f['id']]
            self.assertEqual(len(expected), len(f['utterances']))
            self.assertEqual(len({row['text'] for row in f['utterances']}), len(f['utterances']))
            for index, (source, labels) in enumerate(zip(f['utterances'], expected)):
                self.assertEqual(set(labels), keys)
                self.assertEqual(source['start_ms'], index * 5000)
                self.assertEqual(source['end_ms'], index * 5000 + 4000)
                self.assertEqual(source['speaker'], 'others')
                for kind in corpus.KINDS:
                    self.assertIn(labels[kind], corpus.STATES)
                    self.assertIn(labels[kind + '_support'], corpus.SUPPORT)
                    all_states.add(labels[kind]); all_support.add(labels[kind + '_support'])
        self.assertEqual(all_states, set(corpus.STATES))
        self.assertEqual(all_support, {'supported', 'absent'})
        self.assertEqual(corpus.STATES, previous.STATES)
        self.assertEqual(corpus.SUPPORT, previous.SUPPORT)

    def test_complete_item_gold_matches_sources_and_never_invents_extraction(self):
        for f in self.spec['fixtures']:
            expected = self.spec['expected_answers'][f['id']]
            gold = self.spec['gold_items'][f['id']]
            tuples = set()
            for item in gold:
                index = item['window_index']
                self.assertEqual(item['text'], f['utterances'][index]['text'])
                self.assertEqual(item['commitment'], expected[index][item['kind']])
                self.assertEqual(expected[index][item['kind'] + '_support'], 'supported')
                self.assertNotIn(item['commitment'], ['none', 'negated'])
                self.assertIsNone(item['owner']); self.assertIsNone(item['due'])
                key = (index, item['kind'], item['commitment'])
                self.assertNotIn(key, tuples); tuples.add(key)
            required = {(index, kind, labels[kind]) for index, labels in enumerate(expected)
                        for kind in corpus.KINDS if labels[kind + '_support'] == 'supported' and labels[kind] not in ['none', 'negated']}
            self.assertEqual(tuples, required)

    def test_negative_questions_and_mixed_states_keep_authored_ontology(self):
        self.assertEqual(corpus.CASE_STATES[6], {})
        self.assertEqual(corpus.CASE_STATES[13], {})
        self.assertEqual(corpus.CASE_STATES[14], {'action': 'unknown'})
        self.assertEqual(corpus.CASE_SUPPORT[14]['action'], 'supported')
        for case, kind in [(5, 'decision'), (11, 'action')]:
            self.assertEqual(corpus.CASE_STATES[case][kind], 'negated')
            self.assertEqual(corpus.CASE_SUPPORT[case][kind], 'absent')

    def test_development_panels_and_transport_configuration_are_separate(self):
        by_id = {f['id']: f for f in self.spec['fixtures']}
        for key, length in [('resource_fixture_id', 16), ('stress_fixture_id', 512)]:
            f = by_id[self.spec[key]]
            self.assertEqual(f['split'], 'development')
            self.assertEqual(len(f['utterances']), length)
        self.assertNotIn('configuration', self.spec)
        self.assertNotIn('expected_actual', self.spec)
        config = {'protocol': 'pennel_cloud', 'model': 'pennel-review-v3'}
        actual = {'model': 'a separately verified model'}
        with_route = corpus.experiment(config, actual)
        config['model'] = 'changed'; actual['model'] = 'changed'
        self.assertEqual(with_route['configuration']['model'], 'pennel-review-v3')
        self.assertEqual(with_route['expected_actual']['model'], 'a separately verified model')
        self.assertEqual(self.spec, corpus.experiment())
        self.assertEqual(self.spec['corpus_id'], 'authored-managed-decider-meetings-v1')
        self.assertIn('actual production export', self.spec['provenance']['context_coverage'])


if __name__ == '__main__':
    unittest.main()
