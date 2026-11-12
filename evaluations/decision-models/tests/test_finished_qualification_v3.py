import copy
import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import finished
import finished_corpus_v3 as corpus
import finished_qualification as qualification
import finished_v3
import finished_experiment as actor
import metrics

KINDS=corpus.KINDS
ACTUAL=corpus.experiment()['expected_actual']
CONFIG=corpus.experiment()['configuration']


class SupportBridge:
    artifact_sha256='c'*64
    def __init__(self,batch_size=8):self.batch_size=batch_size
    def __call__(self,request):
        if request['op']=='finished_meeting_export':
            fixture=request['fixture'];windows=[];batches=[]
            for index,row in enumerate(fixture['utterances']):
                source={'meeting_uid':'fixture-meeting','utterance_id':f'{fixture["id"]}-{index}',
                    'source_fingerprint':finished.digest(row),'quote':row['text'],'start_byte':0,
                    'end_byte':len(row['text'].encode()),'start_ms':row['start_ms'],'end_ms':row['end_ms']}
                questions=[{'id':key,'kind':'choice','options':[{'id':label} for label in
                    (('supported','absent','unclear') if key.endswith('_support') else ('none','confirmed'))]}
                    for key in list(KINDS)+[kind+'_support' for kind in KINDS]]
                windows.append({'index':index,'batch_index':index//self.batch_size,'batch_slot':index%self.batch_size,
                    'prompt_context_sha256':finished.digest({'size':min(self.batch_size,len(fixture['utterances'])-index//self.batch_size*self.batch_size),'slot':index%self.batch_size}),
                    'request':{'questions':questions},'sources':[{'source':source}]})
            for start in range(0,len(windows),self.batch_size):
                indices=list(range(start,min(start+self.batch_size,len(windows))))
                mapping=[{'window_index':index,'original_id':question['id'],'batch_id':f'w{index-start}_{question["id"]}'}
                    for index in indices for question in windows[index]['request']['questions']]
                body=json.dumps({'fixture':fixture['id'],'window_indices':indices,'questions':{x['batch_id']:{} for x in mapping}},sort_keys=True)
                batches.append({'index':len(batches),'window_indices':indices,'answer_map':mapping,'wire_body':body,
                    'wire_body_sha256':hashlib.sha256(body.encode()).hexdigest()})
            value={'schema_version':3,'protocol_id':'production-finished-meeting-v3',
                'runtime_policy':'raw_supported_passage_decisions_v3','fixture':fixture,
                'configuration':request['configuration'],'expected_actual':request['expected_actual'],
                'qualification_request':{'task':'meeting_suggestions_v1','language_policy':'nl',
                    'grouping':'fixture-supported-batches','resource_profile_sha256':'9'*64},
                'source_sha256':{'production':'a'*64},'production_admission':False,'windows':windows,'batches':batches}
            value['freeze_sha256']=finished.digest(value);return value
        exported=request['export'];rows=[]
        for response in request['responses']:
            batch=exported['batches'][response['batch_index']];labels=json.loads(response['body'])
            for index in batch['window_indices']:
                window=exported['windows'][index];source=window['sources'][0]['source'];answers=[];choices={};abstained_ids=set()
                for question in window['request']['questions']:
                    key=question['id'];label=labels[f'w{window["batch_slot"]}_{key}'];choices[key]=label
                    abstained=isinstance(label,dict)
                    if abstained:abstained_ids.add(key)
                    chosen=label.get('choice') if abstained else label
                    choices[key]=chosen
                    answers.append({'id':key,'choice':chosen,'abstained':abstained,
                        'probabilities':[{'option_id':option['id'],'probability':float(chosen==option['id'])} for option in question['options']]})
                candidates=[{'kind':kind,'commitment':'confirmed','text':source['quote'],'owner':None,'due':None,'sources':[source]}
                    for kind in KINDS if choices[kind]=='confirmed' and choices[kind+'_support']=='supported' and kind+'_support' not in abstained_ids]
                assessed=[kind for kind in KINDS if choices[kind+'_support']!='unclear' and kind+'_support' not in abstained_ids]
                rows.append({'window_index':index,'status':'ok','result':{'answers':answers},'candidates':candidates,'assessed_kinds':assessed})
        return {'status':'ok','complete':len(request['responses'])==len(exported['batches']),
            'windows':rows,'production_admission':False,'batches':[]}
    def call_before(self,request,deadline):return self(request)


def experiment(bridge,length=16):
    fixtures=[];answers={};gold={}
    for split in ('development','calibration','heldout'):
        for family in range(12):
            identifier=f'v3-{split}-{family}'
            rows=[{'start_ms':i*1000,'end_ms':i*1000+900,'speaker':'others',
                'text':f'Uitsluitend fictieve {split} bron {family} passage {i}.'} for i in range(length)]
            fixtures.append({'id':identifier,'family_id':identifier,'split':split,'language':'nl','utterances':rows})
            labels=[{kind:'confirmed' if (family+i)%2 else 'none' for kind in KINDS} for i in range(length)]
            for row in labels:row.update({kind+'_support':'supported' if row[kind]=='confirmed' else 'absent' for kind in KINDS})
            answers[identifier]=labels
            gold[identifier]=[{'window_index':i,'kind':kind,'commitment':row[kind],'text':rows[i]['text'],'owner':None,'due':None}
                for i,row in enumerate(labels) for kind in KINDS if row[kind]=='confirmed']
    return qualification.freeze(fixtures,CONFIG,bridge,ACTUAL,answers,gold,schema_version=3)


def predictions(frozen,split,evidence='recorded_inference'):
    result=[]
    for exported in frozen['production']['exports']:
        fixture=exported['fixture']
        if fixture['split']!=split:continue
        replies=[]
        for batch in exported['batches']:
            body={entry['batch_id']:frozen['production']['expected_answers'][fixture['id']][entry['window_index']][entry['original_id']]
                for entry in batch['answer_map']}
            replies.append({'batch_index':batch['index'],'http_status':200,'body':json.dumps(body)})
        result.append({'fixture_id':fixture['id'],'freeze_sha256':frozen['freeze_sha256'],'export_sha256':exported['freeze_sha256'],
            'evidence_class':evidence,'status':'ok','reason':None,'spend_ledger':{'limit_usd':'2.00','known_billed_usd':'.01',
                'reserved_usd':'0','pending_reservations':{},'unresolved_charge':False},
            'request_sha256s':[batch['wire_body_sha256'] for batch in exported['batches']],
            'responses':replies,'measurements':{'whole_job_elapsed_ms':100,'executed_requests':len(replies)}})
    return result


def resources(ready):
    run=predictions(ready,'development')[0]
    return {'freeze_sha256':ready['freeze_sha256'],'resource_profile_sha256':'9'*64,
        'scope':'owned_client','cpu_time_ms':100,'peak_rss_bytes':1000,
        'samples':[{'id':f'{phase}-{i}','phase':phase,'run':copy.deepcopy(run)}
            for phase,count in (('cold',5),('warm',30)) for i in range(count)]}


class SupportedQualificationTests(unittest.TestCase):
    def test_actual_export_shape_drives_context_coverage(self):
        bridge=SupportBridge(batch_size=3)
        frozen=experiment(bridge,length=16)
        coverage=frozen['production']['calibration_context_coverage']
        self.assertEqual({row['batch_size'] for row in coverage},{1,3})
        self.assertTrue(all(row['observations']>=10 and row['classes']>=2 and row['families']>=2 for row in coverage))
        changed=copy.deepcopy(frozen['production']['exports'])
        changed[-1]['windows'][0]['prompt_context_sha256']='f'*64
        coverage=finished_v3.context_coverage(changed,frozen['production']['expected_answers'])
        self.assertTrue(any(row['observations']==0 and row['required_splits']==['heldout'] for row in coverage))

    def test_freeze_refuses_an_actual_heldout_context_missing_calibration(self):
        class HeldoutDrift(SupportBridge):
            def __call__(self,request):
                value=super().__call__(request)
                if request['op']=='finished_meeting_export' and request['fixture']['split']=='heldout':
                    value['windows'][0]['prompt_context_sha256']='f'*64
                    value['freeze_sha256']=finished.digest({k:v for k,v in value.items() if k!='freeze_sha256'})
                return value
        with self.assertRaisesRegex(ValueError,'unsupported_actual_prompt_context'):
            experiment(HeldoutDrift())

    def test_state_and_support_are_separately_scored_and_item_f1_excludes_support(self):
        bridge=SupportBridge();base=experiment(bridge)
        ready=qualification.calibrate(base,predictions(base,'calibration'),bridge)
        report=qualification.evaluate(ready,predictions(ready,'heldout'),bridge,resources(ready))
        self.assertTrue(report['qualification']);self.assertEqual(report['gates'],[])
        for key in ('raw_component_metrics','raw_support_metrics','complete_item_metrics'):
            metric=report[key]['classification'] if key=='complete_item_metrics' else report[key]
            self.assertEqual(metric['count'],12*16*4)
            self.assertEqual(metric['macro_f1'],1)
        self.assertTrue(report['runtime_support_check_applied'])
        self.assertFalse(report['runtime_calibration_applied'])
        self.assertFalse(report['candidate_records'][0]['evidence']['foreground_impact_accepted'])

    def test_unsupported_presence_cannot_get_card_credit_or_hide_behind_easy_rows(self):
        bridge=SupportBridge();base=experiment(bridge)
        ready=qualification.calibrate(base,predictions(base,'calibration'),bridge)
        held=predictions(ready,'heldout')
        for run in held:
            for response in run['responses']:
                body=json.loads(response['body'])
                body.update({key:'absent' for key in body if key.endswith('_support')})
                response['body']=json.dumps(body)
        report=qualification.evaluate(ready,held,bridge,resources(ready))
        self.assertEqual(report['raw_component_metrics']['macro_f1'],1)
        self.assertLess(report['complete_item_metrics']['classification']['macro_f1'],.9)
        self.assertIn('support_component_quality_failed',report['gates'])
        self.assertIn('complete_item_quality_failed',report['gates'])
        self.assertEqual(report['candidate_records'],[])

    def test_actor_accepts_only_the_approved_c_corpus_for_v3(self):
        fixture=corpus.experiment()['fixtures'][0]
        actor._approved_fixture(fixture,3)
        with self.assertRaisesRegex(ValueError,'approved_invented_corpus_required'):
            actor._approved_fixture(fixture,2)
        changed=copy.deepcopy(fixture);changed['utterances'][0]['text']='Unapproved text'
        with self.assertRaisesRegex(ValueError,'approved_invented_corpus_required'):
            actor._approved_fixture(changed,3)

    def test_unclear_support_is_an_abstention_with_distribution_retained(self):
        bridge=SupportBridge();base=experiment(bridge);held=predictions(base,'heldout')
        ready=qualification.calibrate(base,predictions(base,'calibration'),bridge)
        for special in ('unclear',{'choice':'supported'}):
            changed=copy.deepcopy(held)
            body=json.loads(changed[0]['responses'][0]['body']);body['w0_action_support']=special
            changed[0]['responses'][0]['body']=json.dumps(body)
            rows,_,_=qualification._runs(base,changed,bridge,'heldout')
            row=next(row for row in rows if row['id']=='v3-heldout-0:0:action_support')
            self.assertIsNone(row['prediction']);self.assertFalse(row['accepted'])
            self.assertTrue(row['support_abstained'])
            self.assertEqual(set(row['probabilities']),{'supported','absent','unclear'})
            diagnostic=qualification._support_abstentions(qualification._calibrated([row],ready['calibration']))[0]
            self.assertIsNone(diagnostic['prediction']);self.assertFalse(diagnostic['accepted'])
            self.assertIsNotNone(diagnostic['probabilities'])
            support_metric=metrics.classification_metrics([value for value in rows if value['component'].endswith('_support')])
            self.assertLess(support_metric['coverage'],1)
            self.assertNotIn('unclear',support_metric['per_class'])
            item_metric=qualification._items(base,qualification._runs(base,changed,bridge,'heldout')[1],rows)
            self.assertLess(item_metric['classification']['coverage'],1)
        mapped=qualification._support_abstentions([{'prediction':'unclear','expected':'supported','probabilities':{'unclear':1}}])
        self.assertIsNone(mapped[0]['prediction']);self.assertFalse(mapped[0]['accepted'])
        state={'prediction':'unknown','expected':'unknown'}
        self.assertEqual(qualification._support_abstentions([state]),[state])


if __name__=='__main__':unittest.main()
