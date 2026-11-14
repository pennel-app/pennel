"""Explicit Cloud-route evaluation using the actual Rust Engine and device session.

Only invented, frozen authored fixtures are accepted. Credentials travel through
anonymous pipes; the existing cumulative ledger is reserved before each call.
"""

import argparse
from decimal import Decimal
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import time
import uuid

import finished
import finished_experiment as common
import finished_qualification as qualification
import finished_support_v1
import finished_corpus_managed_v1 as managed_corpus
from evaluation_io import jsonl_exchange, remaining, stop_owned_group, stop_owned_process

PROFILE = 'supported_passages_v3'
MODEL = 'pennel-review-v3'
ORIGINS = ('https://api.pennel.app', 'https://api-staging.pennel.app')
MAX_SESSION_BYTES = 8192
MIN_RESERVATION = Decimal('.10')


def session_from_fd(fd):
    if isinstance(fd, bool) or not isinstance(fd, int) or fd < 3:
        raise ValueError('anonymous_session_pipe_required')
    data = bytearray(); deadline = time.monotonic()+15
    try:
        if not common._anonymous_credential_pipe(fd, os.fstat(fd)):
            raise ValueError('anonymous_session_pipe_required')
        os.set_blocking(fd, False)
        with selectors.DefaultSelector() as selector:
            selector.register(fd, selectors.EVENT_READ)
            while True:
                remaining=deadline-time.monotonic()
                if remaining<=0 or not selector.select(remaining):raise ValueError('session_pipe_timeout')
                try:part=os.read(fd, MAX_SESSION_BYTES+1-len(data))
                except BlockingIOError:continue
                if not part:break
                data.extend(part)
                if len(data)>MAX_SESSION_BYTES:raise ValueError('session_pipe_too_large')
        value=json.loads(data)
        if (set(value)!={'schema_version','access_token_only','origin','access_token','expires_at_unix_seconds'}
                or type(value['schema_version']) is not int or value['schema_version']!=1 or value['access_token_only'] is not True
                or value['origin'] not in ORIGINS or not isinstance(value['access_token'],str)
                or not value['access_token'] or isinstance(value['expires_at_unix_seconds'],bool)
                or not isinstance(value['expires_at_unix_seconds'],int)):
            raise ValueError('invalid_access_only_session')
        return bytes(data)
    except (OSError,ValueError,TypeError,KeyError):
        raise ValueError('invalid_access_only_session') from None
    finally:
        for index in range(len(data)):data[index]=0
        os.close(fd)


def _spawn(arguments, session, *, text=False, new_group=False):
    if len(session)>MAX_SESSION_BYTES:raise ValueError('session_pipe_too_large')
    reader,writer=os.pipe()
    environment={key:value for key,value in os.environ.items()
                 if key in ('PATH','HOME','TMPDIR','LANG','LC_ALL','SYSTEMROOT')}
    process=None
    try:
        process=subprocess.Popen(arguments+['--session-fd',str(reader)],pass_fds=(reader,),
            stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,env=environment,
            text=text,bufsize=1 if text else -1,start_new_session=new_group)
        os.close(reader);reader=None
        with os.fdopen(writer,'wb') as pipe:writer=None;pipe.write(session)
        return process
    except BaseException:
        if process is not None:
            if process.poll() is None:
                if new_group:stop_owned_group(process)
                else:process.kill()
            process.wait()
        raise
    finally:
        if reader is not None:os.close(reader)
        if writer is not None:os.close(writer)


class CloudClient:
    def __init__(self,binary,session):
        self.binary=str(Path(binary).resolve(strict=True));self.sha256=finished.file_hash(self.binary)
        self.process=_spawn([self.binary,'--execute-cloud-review'],session)
        self.buffer=bytearray()
    def call(self,value,deadline):
        try:
            remaining(deadline,'cloud_client')
            if finished.file_hash(self.binary)!=self.sha256:raise ValueError('bridge_binary_changed')
            result=jsonl_exchange(self.process,value,self.buffer,deadline,prefix='cloud_client',
                max_response_bytes=finished.MAX_RESPONSE_BYTES*8)
            if not isinstance(result,dict):raise ValueError('cloud_client_invalid_output')
            if 'error' in result:raise ValueError(result['error'])
            remaining(deadline,'cloud_client')
            return result
        except BaseException:
            stop_owned_process(self.process)
            raise
    def close(self):
        if not self.process.stdin.closed:
            try:self.process.stdin.close()
            except BrokenPipeError:pass
        try:self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:self.process.kill();self.process.wait()
        self.process.stdout.close()
    def __enter__(self):return self
    def __exit__(self,*_):self.close()


def route(exported):
    config=exported['configuration'];actual=exported['expected_actual']
    if (config.get('protocol')!='pennel_cloud' or config.get('model')!=MODEL
            or config.get('endpoint') not in tuple(origin+'/v1/decisions' for origin in ORIGINS)
            or not qualification._valid_actual(config,actual)
            or exported.get('runtime_policy') not in ('raw_supported_passage_decisions_v3', finished_support_v1.RUNTIME_POLICY)):
        raise ValueError('frozen_managed_review_route_required')
    if exported.get('runtime_policy') == finished_support_v1.RUNTIME_POLICY:
        qualification.finished_calibration_reuse.metadata(exported)
    return config


def authored_spec(path,frozen):
    raw=Path(path).read_bytes();spec=json.loads(raw)
    if qualification._support(frozen):
        import finished_corpus_managed_v2
        canonical=finished_corpus_managed_v2.experiment()
    else:canonical=managed_corpus.experiment()
    if any(spec.get(key)!=canonical[key] for key in ('fixtures','expected_answers','gold_items')):
        raise ValueError('canonical_fresh_managed_corpus_required')
    if frozen.get('resource_fixture_id')!=canonical['resource_fixture_id']:
        raise ValueError('canonical_managed_resource_panel_required')
    by_id={item['id']:item for item in spec['fixtures']}
    exports=frozen['production']['exports']
    if len(by_id)!=len(exports) or spec['expected_answers']!=frozen['production']['expected_answers']:
        raise ValueError('authored_cloud_corpus_changed')
    for exported in exports:
        fixture=exported['fixture'];identifier=fixture['id']
        if not identifier.startswith(('nl-cloud-','nl-managed-v1-','nl-managed-v2-')) or by_id.get(identifier)!=fixture:
            raise ValueError('fresh_authored_cloud_fixture_required')
        if qualification._gold(exported,spec['gold_items'][identifier])!=frozen['gold_items'][identifier]:
            raise ValueError('authored_cloud_judgment_changed')
    old_held={row['text'].strip() for fixture in common.corpus_v3.experiment()['fixtures']
              if fixture['split']=='heldout' for row in fixture['utterances']}
    if any(row['text'].strip() in old_held for fixture in spec['fixtures']
           if fixture['split'] in ('calibration','heldout') for row in fixture['utterances']):
        raise ValueError('old_heldout_text_is_not_fresh_cloud_evidence')
    return hashlib.sha256(raw).hexdigest()


def validate_attempt(exported,batch,attempt):
    actual=json.loads(attempt['wire_body'],object_pairs_hook=common.runners._unique_json)
    request_id=actual.pop('request_id',None)
    if (not isinstance(request_id,str) or uuid.UUID(request_id).int==0
            or actual!=json.loads(batch['wire_body']) or attempt['endpoint']!=exported['configuration']['endpoint']
            or hashlib.sha256(attempt['wire_body'].encode()).hexdigest()!=attempt.get('wire_body_sha256')
            or attempt.get('raw_http_bytes_available') is not False
            or attempt.get('response_capture')!='parsed_service_json_before_decision_decoder'):
        raise ValueError('actual_managed_request_changed')
    upstream=dict(actual);upstream.pop('expected_model')
    upstream.update(model=common.MODEL,provider={'only':['perplexity'],'allow_fallbacks':False,'data_collection':'deny','zdr':True})
    if upstream!=batch['managed_transport']['canonical_upstream_contract']:
        raise ValueError('canonical_upstream_contract_changed')
    return request_id


def run_job(exported,freeze_sha256,bridge,session,ledger,output,*,execute=False,
            global_deadline=None,prices=(Decimal('.00000002'),Decimal(0)),client_factory=CloudClient):
    if not execute:raise ValueError('execution_not_enabled')
    config=route(exported);output=Path(output)
    if output.exists():raise ValueError('preserve_existing_execution_artifact')
    if ledger.limit>Decimal(2):raise ValueError('budget_exceeds_usd_2')
    started=time.monotonic();initial=common._usage()
    deadline=started+config['timeout_secs']
    if global_deadline is not None:deadline=min(deadline,global_deadline)
    run={'fixture_id':exported['fixture']['id'],'split':exported['fixture']['split'],'freeze_sha256':freeze_sha256,
         'export_sha256':exported['freeze_sha256'],'evidence_class':'recorded_inference','requests':[],
         'request_sha256s':[],'actual_request_sha256s':[],'responses':[],'status':'blocked','reason':None,
         'session_mode':'access_token_only','refresh_available':False,
         'response_capture':'parsed_service_json_before_decision_decoder','raw_http_bytes_available':False,
         'transport_lifecycle':'one_fresh_Rust_Engine_per_original_job;retained_over_all_physical_batches',
         'remote_server_cpu_rss':'unavailable','remote_server_warmth':'unknown'}
    def finish(reason):
        final=common._usage();run.update(status='ok' if reason is None else 'blocked',reason=reason)
        run['measurements']={'whole_job_elapsed_ms':(time.monotonic()-started)*1000,
            'cpu_time_ms':max(0,final['cpu_ms']-initial['cpu_ms']),'peak_rss_bytes':final['peak_rss_bytes'],
            'client_peak_rss_bytes':final['client_peak_rss_bytes'],'largest_child_peak_rss_bytes':final['largest_child_peak_rss_bytes'],
            'aggregate_process_tree_rss_bytes':None,'rss_measurement':'maximum_individual_process_lifetime_high_water_mark',
            'executed_requests':sum(len(row.get('attempts',[])) for row in run['requests']),
            'input_bytes':sum(len(attempt['wire_body'].encode()) for row in run['requests'] for attempt in row.get('attempts',[]))}
        run['spend_ledger']=ledger.snapshot();common.write_json(output,run);return run
    if ledger.unresolved or ledger.pending:return finish('unreconciled_existing_charge')
    finished._verify_export(exported,bridge,deadline=deadline)
    reason=None;run_id=str(uuid.uuid4())
    with client_factory(bridge.binary,session) as client:
        if client.sha256!=bridge.artifact_sha256:raise ValueError('bridge_binary_changed')
        for batch in qualification.physical_requests(exported):
            if time.monotonic()>=deadline:reason='job_deadline_exceeded';break
            bound=max(MIN_RESERVATION,common.reservation_bound(batch['wire_body'],*prices))
            if ledger.total_exposure+bound+common.NATIVE_SMOKE_ALLOWANCE>ledger.limit:
                reason='reserved_native_smoke_budget_unavailable';break
            template_sha=hashlib.sha256(batch['wire_body'].encode()).hexdigest()
            token=ledger.reserve(bound,{'fixture_id':run['fixture_id'],'batch_index':batch['index'],'request_sha256':template_sha})
            entry={'batch_index':batch['index'],'reservation_id':token,'template_request_sha256':template_sha,'attempts':[]}
            run['requests'].append(entry);common.write_json(output,run)
            command={'op':'finished_cloud_review_batch','run_id':run_id,'batch_index':batch['index']}
            if batch['index']==0:command['export']=exported
            else:command['export_sha256']=exported['freeze_sha256']
            settlement_attempted=False
            try:
                value=client.call(command,deadline)
                entry['attempts']=value.get('attempts',[]);entry['outcome']=value.get('outcome',{})
                if len(entry['attempts'])!=1:raise ValueError('managed_attempt_count_changed')
                attempt=entry['attempts'][0];request_id=validate_attempt(exported,batch,attempt)
                response=attempt.get('response');raw=json.dumps(response,ensure_ascii=False,allow_nan=False) if isinstance(response,dict) else ''
                status=attempt.get('http_status');entry.update(request_id=request_id,http_status=status,raw_response=raw)
                settlement_attempted=True
                ledger.settle(token,common._cost(raw),{'fixture_id':run['fixture_id'],'batch_index':batch['index'],
                    'request_sha256':attempt['wire_body_sha256'],'http_status':status})
                token=None
                run['request_sha256s'].append(template_sha);run['actual_request_sha256s'].append(attempt['wire_body_sha256'])
                recorded_response={'batch_index':batch['index'],'http_status':status,'body':raw,'request_id':request_id}
                if exported.get('runtime_policy') == finished_support_v1.RUNTIME_POLICY:
                    production_result=entry['outcome'].get('result')
                    if not isinstance(production_result,dict):raise ValueError('recorded_engine_result_required')
                    recorded_response['production_result']=production_result
                run['responses'].append(recorded_response)
                common.write_json(output,run)
                if ledger.unresolved:reason='unresolved_charge';break
                if entry['outcome'].get('status')!='ok':reason=entry['outcome'].get('status','cloud_request_failed');break
                if entry['outcome'].get('replay',{}).get('status')!='ok':reason=entry['outcome']['replay'].get('status','invalid_replay');break
            except Exception:
                if token is not None and not settlement_attempted:ledger.settle(token,None,{'fixture_id':run['fixture_id'],'batch_index':batch['index'],'reason':'cloud_attempt_unreconciled'})
                reason='cloud_execution_failed';break
    if reason is None and time.monotonic()>=deadline:reason='job_deadline_exceeded'
    return finish(reason)


class JobClient(common.JobClient):
    def __init__(self,session):
        self.grace_seconds=10;self.started=time.monotonic()
        self.process=_spawn([sys.executable,str(Path(__file__).resolve()),'_client','--execute'],session,new_group=True)
        self.buffer=bytearray()
    def call(self,value):
        deadline=min(value['global_deadline'],time.monotonic()+value['export']['configuration']['timeout_secs']+self.grace_seconds)
        try:
            result=jsonl_exchange(self.process,value,self.buffer,deadline,prefix='client',
                max_response_bytes=finished.MAX_RESPONSE_BYTES*8)
            if not isinstance(result,dict) or result.get('status') not in ('ok','blocked'):
                raise ValueError('invalid_client_response')
            remaining(deadline,'client')
            return result
        except BaseException:
            stop_owned_group(self.process)
            raise
    def close(self):
        if not self.process.stdin.closed:
            try:self.process.stdin.close()
            except BrokenPipeError:pass
        try:self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:stop_owned_group(self.process)
        self.process.stdout.close()


def client_worker(fd):
    session=session_from_fd(fd)
    for line in sys.stdin:
        value=json.loads(line);bridge=finished.CoreBridge(value['bridge'])
        if bridge.artifact_sha256!=value['bridge_sha256']:raise ValueError('bridge_binary_changed')
        ledger=common.load_ledger(value['ledger'])
        result=run_job(value['export'],value['freeze_sha256'],bridge,session,ledger,value['output'],execute=True,
            global_deadline=value['global_deadline'],prices=tuple(Decimal(price) for price in value['prices']))
        print(json.dumps({'status':result['status'],'reason':result['reason']}),flush=True)


def select_jobs(phase,jobs,start=0,count=None):
    # Explicit disjoint selections fit short access leases. Final calibration/scoring still
    # requires every frozen fixture; a completed selection is never a completed phase.
    if (type(start) is not int or start<0 or (count is not None and (type(count) is not int or count<1))
            or (phase not in ('calibration','heldout') and (start!=0 or count is not None))):
        raise ValueError('invalid_explicit_job_selection')
    end=len(jobs) if count is None else start+count
    if start>=len(jobs) or end>len(jobs):raise ValueError('invalid_explicit_job_selection')
    return jobs[start:end],{'start':start,'end_exclusive':end,'total_frozen_jobs':len(jobs),
                            'entire_phase':start==0 and end==len(jobs)}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase',choices=('preflight','development','calibration','heldout','resources','_client'))
    parser.add_argument('--bridge');parser.add_argument('--freeze');parser.add_argument('--authored-spec')
    parser.add_argument('--pricing');parser.add_argument('--ledger',default=str(common.DEFAULT_LEDGER))
    parser.add_argument('--output');parser.add_argument('--execute',action='store_true');parser.add_argument('--session-fd',type=int,required=True)
    parser.add_argument('--fixture-id');parser.add_argument('--freeze-lock');parser.add_argument('--freeze-commit')
    parser.add_argument('--job-start',type=int,default=0);parser.add_argument('--job-count',type=int)
    args=parser.parse_args()
    if not args.execute:raise ValueError('execution_not_enabled')
    if args.phase=='_client':client_worker(args.session_fd);return
    output=Path(args.output);output.mkdir(parents=True,exist_ok=False)
    bridge=finished.CoreBridge(args.bridge)
    if args.phase=='preflight':
        session=session_from_fd(args.session_fd)
        with CloudClient(args.bridge,session) as client:result=client.call({'op':'cloud_review_preflight'},time.monotonic()+35)
        common.write_json(output/'preflight.json',result);print(json.dumps({'preflight_complete':True,'provider_requests':0}));return
    frozen=json.loads(Path(args.freeze).read_text());qualification.verify_freeze(frozen,bridge)
    if not qualification._cloud(frozen):raise ValueError('cloud_qualification_freeze_required')
    if args.phase=='calibration' and qualification._support(frozen):
        raise ValueError('calibration_reused_without_new_inference')
    spec_sha=authored_spec(args.authored_spec,frozen)
    if args.phase!='development':
        common._verify_committed_lock(args.freeze_commit,args.freeze_lock,frozen)
        relative=Path(args.freeze_lock).resolve().relative_to(common.ROOT)
        lock=json.loads(subprocess.check_output(['git','show',args.freeze_commit+':'+str(relative)],cwd=common.ROOT))
        if lock.get('authored_spec_sha256')!=spec_sha:raise ValueError('committed_authored_cloud_corpus_required')
    if args.phase in ('heldout','resources') and frozen['stage']!='heldout_ready':raise ValueError('calibration_before_heldout_required')
    prices=common._prices(args.pricing)
    exports=frozen['production']['exports']
    for exported in exports:route(exported)
    if args.phase=='development':
        if not args.fixture_id:raise ValueError('explicit_development_fixture_required')
        selected=[x for x in exports if x['fixture']['id']==args.fixture_id and x['fixture']['split']=='development']
        if len(selected)!=1:raise ValueError('development_fixture_not_frozen')
        jobs=[(None,None,selected[0])]
    elif args.phase=='resources':
        panel=next(x for x in exports if x['fixture']['id']==frozen['resource_fixture_id'])
        jobs=[(f'cold-{i}','cold',panel) for i in range(5)]+[(f'warm-{i}','warm',panel) for i in range(30)]
    else:jobs=[(None,None,x) for x in exports if x['fixture']['split']==args.phase]
    jobs,selection=select_jobs(args.phase,jobs,args.job_start,args.job_count)
    session=session_from_fd(args.session_fd);started=time.monotonic();deadline=started+common.MAX_SECONDS
    runs=[];samples=[]
    with common.locked_ledger(args.ledger):
        warm=None
        try:
            for key,phase,exported in jobs:
                target=output/(str(key or exported['fixture']['id'])+'.json')
                fresh=phase=='cold'
                client=JobClient(session) if fresh else warm
                if client is None:warm=client=JobClient(session)
                try:
                    result=client.call({'export':exported,'freeze_sha256':frozen['freeze_sha256'],'bridge':args.bridge,
                        'bridge_sha256':bridge.artifact_sha256,'ledger':args.ledger,'output':str(target),
                        'global_deadline':deadline,'prices':[str(price) for price in prices]})
                finally:
                    if fresh:client.close()
                run=json.loads(target.read_text());runs.append(run)
                if key:samples.append({'id':key,'phase':phase,'run':run})
                if result['status']!='ok':break
        finally:
            if warm:warm.close()
    if args.phase=='resources':
        profile=jobs[0][2]['qualification_request']['resource_profile_sha256']
        value={'freeze_sha256':frozen['freeze_sha256'],'resource_profile_sha256':profile,'scope':'owned_client','samples':samples,
            'cpu_time_ms':sum(run['measurements']['cpu_time_ms'] for run in runs),
            'peak_rss_bytes':max((run['measurements']['peak_rss_bytes'] for run in runs),default=0)}
        common.write_json(output/'resources.json',value)
    else:common.write_json(output/'predictions.json',runs)
    complete=len(runs)==len(jobs) and all(x['status']=='ok' for x in runs)
    common.write_json(output/'phase-receipt.json',{'phase':args.phase,'complete':complete and selection['entire_phase'],
        'selection_complete':complete,'selection':selection,
        'scheduled_jobs':len(jobs),'executed_jobs':len(runs),'freeze_sha256':frozen['freeze_sha256'],'authored_spec_sha256':spec_sha,
        'session_mode':'access_token_only','no_refresh_token':True,'ledger_after':common.load_ledger(args.ledger).snapshot()})
    print(json.dumps({'phase':args.phase,'complete':complete and selection['entire_phase'],
        'selection_complete':complete,'selection':selection,'executed_jobs':len(runs),'scheduled_jobs':len(jobs)}))
    if not complete:raise SystemExit(1)


if __name__=='__main__':main()
