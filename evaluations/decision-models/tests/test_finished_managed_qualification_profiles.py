import copy
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import finished_qualification as qualification
import finished_v3


class ManagedLocalQualificationTests(unittest.TestCase):
    def setUp(self):
        digest='ffb27ade8976765b0e6943a0b7f5f3fd0f7042d45ad90256a76d608bcd3942c2'
        self.config={'protocol':'local_decider','endpoint':'pennel-local://decider/decisions',
            'model':'pennel-decider-flash-v1.2-q8_0','expected_model':'sha256:'+digest,'upstream':None,
            'max_input_bytes':65536,'timeout_secs':90}
        self.actual={'model':'sha256:'+digest,'artifact_sha256':digest,'provider':'Pennel local Decider',
            'revision':'a'*64,'identity_evidence':'serving_revision_and_artifact'}
    def test_local_profile_uses_v3_support_semantics_with_exact_loaded_artifact_and_runtime(self):
        self.assertTrue(qualification._valid_actual(self.config,self.actual))
        frozen={'schema_version':3,'protocol_id':'production-finished-qualification-local-v1'}
        self.assertTrue(qualification._v3(frozen));self.assertTrue(qualification._local(frozen))
        self.assertIs(qualification._contract(frozen),finished_v3)
        for key in ('model','artifact_sha256','revision','provider','identity_evidence'):
            invalid=copy.deepcopy(self.actual);invalid[key]='unverified-configured-value'
            self.assertFalse(qualification._valid_actual(self.config,invalid))
    def test_local_protocol_keeps_every_original_quality_and_budget_gate(self):
        root=Path(qualification.__file__).parent
        original=json.loads((root/'protocol-finished-qualification-v3.json').read_text())
        local=json.loads((root/'protocol-finished-qualification-local-v1.json').read_text())
        self.assertEqual(local['acceptance'],original['acceptance'])
        self.assertEqual(local['bounds'],original['bounds'])
    def test_local_artifact_profile_cannot_be_redirected_to_a_custom_server(self):
        invalid=dict(self.config,endpoint='http://127.0.0.1:8080/decisions')
        self.assertFalse(qualification._valid_actual(invalid,self.actual))


if __name__=='__main__':unittest.main()
