#!/usr/bin/env python3
"""Five synthetic identities, owned resource-capped PostgreSQL/app containers only.

Build the pilot Docker target first. Secrets remain in private temporary env files;
output contains timings/counts/bytes only. Never points at an existing database.
"""
import argparse,base64,concurrent.futures,hashlib,json,os,random,secrets,shutil,socket,subprocess,tempfile,threading,time,urllib.error,urllib.parse,urllib.request,uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OPENSSL=os.environ.get('WK_TEST_OPENSSL') or ('/opt/homebrew/opt/openssl@3/bin/openssl' if Path('/opt/homebrew/opt/openssl@3/bin/openssl').exists() else shutil.which('openssl'))
def command(*args,input=None,check=True):
    p=subprocess.run(args,input=input,capture_output=True,text=True)
    if check and p.returncode:raise RuntimeError('Owned pilot command failed: '+p.stderr[:250])
    return p
def b64(raw):return base64.urlsafe_b64encode(raw).rstrip(b'=').decode()
def allocate():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
def main():
    parser=argparse.ArgumentParser();parser.add_argument('--image',default='wk-hosted-pilot:oct10-owned');parser.add_argument('--duration',type=int,default=30);parser.add_argument('--cycle-pause-ms',type=int,default=50);parser.add_argument('--server-binary',type=Path,default=ROOT/'target/debug/wk-server',help='Host wk-server that migrates and provisions; must match the image schema.');parser.add_argument('--deploy-scripts',type=Path,default=ROOT/'deploy/scripts',help='Role and backup scripts matching the image schema.');parser.add_argument('--identities',type=int,default=5);parser.add_argument('--rare-term-search',action='store_true',help='Also search for a term no card contains each cycle, the worst case for lexical search; reported as search_rare.');parser.add_argument('--rare-term-every',type=int,default=0,help='Run the rare-term search once per this many cycles per identity (--rare-term-search means 1).');parser.add_argument('--tolerate-errors',action='store_true',help='Count failed requests in errors and keep the load running (for soaks) instead of aborting.');parser.add_argument('--database-events',action='store_true',help='Log PostgreSQL checkpoints and autovacuum and record their lines (no statement text) with minute offsets, to explain latency stalls.');parser.add_argument('--statement-stats',action='store_true',help='Load pg_stat_statements in the disposable database and record statement shapes and timings.');parser.add_argument('--output',type=Path,required=True);args=parser.parse_args();args.rare_term_every=args.rare_term_every or int(args.rare_term_search);assert args.rare_term_every>=0 and 5<=args.duration<=7200 and 10<=args.cycle_pause_ms<=10000 and 5<=args.identities<=100
    tag=uuid.uuid4().hex;network='wk-pilot-'+tag;database='wk-pilot-db-'+tag;application='wk-pilot-app-'+tag;dbname='wk_pilot_'+tag;role='wk_pilot_runtime';operator_password=secrets.token_hex(24);runtime_password=secrets.token_hex(24);dbport=allocate();port=allocate();origin='http://127.0.0.1:'+str(port);created=[];network_created=False;errors=[];timings={};buckets={};origin_time=time.monotonic();samples=[];lock=threading.Lock();stop=threading.Event()
    def sql(query):
        return command('docker','exec','-i',database,'psql','-X','-At','-v','ON_ERROR_STOP=1','-U','wk_operator','-d',dbname,input=query).stdout.strip()
    def literal(s):return "'"+s.replace("'","''")+"'"
    def cli(mode,value=None):
        env=dict(os.environ,WK_DATABASE_URL='postgresql://wk_operator:'+operator_password+'@127.0.0.1:'+str(dbport)+'/'+dbname)
        p=subprocess.run([str(args.server_binary),mode],env=env,input=None if value is None else json.dumps(value),capture_output=True,text=True,timeout=30)
        assert p.returncode==0,'Synthetic operator '+mode+' failed: '+('storage unavailable' if p.stderr.strip()=='storage unavailable' else 'opaque failure');return p.stdout.strip()
    def metrics():
        return json.loads(sql("SELECT json_build_object('database_bytes',pg_database_size(current_database()),'wal_bytes',(SELECT wal_bytes FROM pg_stat_wal),'notes',(SELECT count(*) FROM personal_summaries),'cards',(SELECT count(*) FROM cards),'nonce_rows',(SELECT count(*) FROM device_nonces),'max_nonces_per_device',COALESCE((SELECT max(n) FROM (SELECT count(*) n FROM device_nonces GROUP BY device_id) q),0),'tables',(SELECT json_object_agg(relname,json_build_object('total_bytes',pg_total_relation_size(relid),'index_bytes',pg_indexes_size(relid))) FROM pg_stat_user_tables))"))
    def sample():
        row={'at':time.monotonic()}
        for label,name in [('app',application),('database',database)]:
            text=command('docker','exec',name,'sh','-c','cat /proc/uptime /sys/fs/cgroup/cpu.stat /sys/fs/cgroup/memory.current /sys/fs/cgroup/memory.peak /proc/1/status').stdout.splitlines();usage=int(next(x.split()[1] for x in text if x.startswith('usage_usec ')));numbers=[int(x) for x in text if x.isdigit()];rss=int(next(x.split()[1] for x in text if x.startswith('VmRSS:')))*1024;row[label]={'observed_at':float(text[0].split()[0]),'throttled_microseconds':int(next(x.split()[1] for x in text if x.startswith('throttled_usec '))),'cpu_microseconds':usage,'memory_bytes':numbers[0],'peak_memory_bytes':numbers[1],'process_rss_bytes':rss}
        row['runtime_pool']=json.loads(sql("SELECT json_build_object('connections',count(*),'active',count(*) FILTER(WHERE state='active'),'lock_waits',count(*) FILTER(WHERE wait_event_type='Lock')) FROM pg_stat_activity WHERE application_name='wk_pilot_runtime'"));samples.append(row)
    def sampler():
        while not stop.wait(1):
            try:sample()
            except Exception:errors.append({'operation':'metrics','error':'sampling_failure'})
    def request(name,path,token=None,value=None,method=None,headers=None,record=True,expected=200):
        body=None if value is None else json.dumps(value,separators=(',',':')).encode();h={'Content-Type':'application/json','Accept':'application/json, text/event-stream'};h.update(headers or {})
        if token:h['Authorization']='Bearer '+token
        began=time.monotonic()
        try:r=urllib.request.urlopen(urllib.request.Request(origin+path,data=body,headers=h,method=method),timeout=35)
        except urllib.error.HTTPError as e:r=e
        except Exception:
            with lock:errors.append({'operation':name,'error':'transport'})
            raise
        raw=r.read();elapsed=(time.monotonic()-began)*1000
        if record:
            with lock:
                timings.setdefault(name,[]).append(elapsed);buckets.setdefault(int((time.monotonic()-origin_time)//60),{}).setdefault(name,[]).append(elapsed)
                if r.code!=expected:errors.append({'operation':name,'status':r.code})
        assert r.code==expected,'Synthetic '+name+' returned '+str(r.code)
        return json.loads(raw) if raw else {}
    try:
        command('docker','image','inspect',args.image)
        with tempfile.TemporaryDirectory(prefix='wk-hosted-pilot-') as temp:
            work=Path(temp);work.chmod(0o700)
            def private(name,data):p=work/name;p.write_text(data);p.chmod(0o600);return p
            db_env=private('db.env','POSTGRES_DB='+dbname+'\nPOSTGRES_USER=wk_operator\nPOSTGRES_PASSWORD='+operator_password+'\n')
            command('docker','network','create',network);network_created=True
            created.append(database)
            command('docker','run','-d','--name',database,'--network',network,'--network-alias','database','--cpus','2','--memory','2g','--pids-limit','256','--env-file',str(db_env),'-p','127.0.0.1:'+str(dbport)+':5432','postgres:17-alpine','postgres','-c','max_connections=50','-c','shared_buffers=256MB','-c','work_mem=4MB',*(['-c','shared_preload_libraries=pg_stat_statements','-c','pg_stat_statements.track=all'] if args.statement_stats else []),*(['-c','log_checkpoints=on','-c','log_autovacuum_min_duration=0','-c','log_line_prefix=%m '] if args.database_events else []))
            for _ in range(100):
                # The image initializes through a temporary UNIX-only server first.
                if command('docker','exec',database,'pg_isready','-h','127.0.0.1','-U','wk_operator','-d',dbname,check=False).returncode==0:break
                time.sleep(.1)
            cli('migrate')
            sql('CREATE ROLE '+role+' LOGIN PASSWORD '+literal(runtime_password))
            sql((args.deploy_scripts/'application-role.sql').read_text().replace('wk_app',role))
            if args.statement_stats:sql('CREATE EXTENSION pg_stat_statements')
            def service(i):return 'frontend' if i%5==2 else 'backend'
            for i in range(args.identities):
                services=['backend','frontend'] if i==0 else [service(i)]
                cli('provision',{'issuer':'http://127.0.0.1:41417','subject':'developer'+str(i),'org':'synthetic-pilot','user':'developer'+str(i),'product':'demo','services':services,'can_review':i==0})
            cli('admin-assign',{'org':'synthetic-pilot','user':'developer1','product':'demo','services':['backend']})
            fixture=json.loads((ROOT/'crates/server/tests/fixtures/synthetic-rsa-a.json').read_text());rsa=private('rsa.pem',fixture['private_pkcs8_pem']);jwks=json.dumps({'keys':[{'kty':'RSA','kid':'synthetic-pilot','alg':'RS256','use':'sig','n':fixture['n'],'e':fixture['e']}]},separators=(',',':'))
            app_env=private('app.env','WK_DATABASE_URL=postgresql://'+role+':'+runtime_password+'@database:5432/'+dbname+'?application_name=wk_pilot_runtime\nWK_PILOT_PUBLIC_JWKS='+jwks+'\nWK_PILOT_ORIGIN='+origin+'\nWK_MCP_RESOURCE=https://pilot.synthetic.example\n')
            created.append(application)
            command('docker','run','-d','--name',application,'--network',network,'--cpus','2','--memory','1g','--pids-limit','256','--read-only','--tmpfs','/tmp','--cap-drop','ALL','--security-opt','no-new-privileges:true','--env-file',str(app_env),'-p','127.0.0.1:'+str(port)+':8080',args.image)
            for _ in range(100):
                try:request('ready','/health/ready',record=False);break
                except Exception:time.sleep(.1)
            else:raise RuntimeError('Synthetic app readiness failed')
            command('docker','exec',application,'wk-server','healthcheck')
            clients=[]
            def jwt(subject,audience):
                now=int(time.time());message=b64(json.dumps({'alg':'RS256','kid':'synthetic-pilot'}).encode())+'.'+b64(json.dumps({'iss':'http://127.0.0.1:41417','aud':audience,'sub':subject,'iat':now,'exp':now+600,'jti':secrets.token_hex(12)}).encode());signed=subprocess.check_output([OPENSSL,'dgst','-sha256','-sign',str(rsa)],input=message.encode(),stderr=subprocess.DEVNULL);return message+'.'+b64(signed)
            for i in range(args.identities):
                folder=work/str(i);folder.mkdir(mode=0o700);pem=folder/'key.pem';command(OPENSSL,'genpkey','-algorithm','ED25519','-out',str(pem));pem.chmod(0o600)
                clients.append({'i':i,'pem':pem,'message':folder/'message','device':str(uuid.uuid4()),'approval':secrets.token_hex(32),'public':subprocess.check_output([OPENSSL,'pkey','-in',str(pem),'-pubout','-outform','DER'],stderr=subprocess.DEVNULL)[-32:].hex(),'collector':jwt('developer'+str(i),'wk-pilot-collector'),'api':jwt('developer'+str(i),'https://pilot.synthetic.example'),'renewed_at':time.monotonic(),'service':service(i)})
            def signed(client,operation,value,record=True):
                value=value if operation=='register' else {'product':'demo','approval_hash':client['approval'],'request':value};body=json.dumps(value,separators=(',',':')).encode();now=int(time.time());nonce=str(uuid.uuid4());path='/collector/'+operation;message=json.dumps(['wk-device-v1',origin,'POST',path,now,nonce,client['device'],hashlib.sha256(client['collector'].encode()).hexdigest(),hashlib.sha256(body).hexdigest()],separators=(',',':')).encode();client['message'].write_bytes(message);client['message'].chmod(0o600);signature=subprocess.check_output([OPENSSL,'pkeyutl','-sign','-rawin','-inkey',str(client['pem']),'-in',str(client['message'])],stderr=subprocess.DEVNULL).hex();return request('collector_'+operation,path,client['collector'],value,headers={'X-WK-Device':client['device'],'X-WK-Time':str(now),'X-WK-Nonce':nonce,'X-WK-Signature':signature},record=record)
            for c in clients:
                signed(c,'register',{'id':c['device'],'product':'demo','services':[c['service']],'public_key':c['public'],'approval_hash':c['approval']},False)
                request('settings','/v1/products/demo/devices/'+c['device']+'/settings',c['api'],{'expected_revision':1,'settings':{'paused':False,'owner_sync':True,'activity':False}},method='PATCH',record=False)
            # Verify actual restricted-service and private-owner isolation before load.
            for c in clients:request('mcp','/mcp/demo',c['api'],{'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'knowledge.search','arguments':{'query':'middleware','limit':8}}},record=False)
            baseline=metrics();sample();monitor=threading.Thread(target=sampler,daemon=True);monitor.start()
            words='component module branch service framework middleware scoped summary decision evidence storage query development reviewed context integration request boundary client handler runtime index transaction deployment testing'.split()
            def body(c,key):
                rng=random.Random(c['i']);text=' '.join(rng.choice(words) for _ in range(1600));value={'schema_version':1,'idempotency_key':key,'policy_revision':1,'title':'Pilot middleware summary','intent':'Bound development context','rationale':'','decisions':[],'constraints':[],'open_questions':[],'service_ids':[c['service']],'evidence':[],'lifecycle':'proposed','capture_completeness':'complete','base_revision':None};length=len(json.dumps(value,separators=(',',':')).encode());value['rationale']=text[:10240-length];assert len(json.dumps(value,separators=(',',':')).encode())==10240;return value
            def refresh(c):
                if time.monotonic()-c['renewed_at']>450:
                    c['collector']=jwt('developer'+str(c['i']),'wk-pilot-collector');c['api']=jwt('developer'+str(c['i']),'https://pilot.synthetic.example');c['renewed_at']=time.monotonic()
            def cycle(c,key,record):
                refresh(c);refresh(clients[0])
                signed(c,'policy',{},record);value=body(c,key);first=signed(c,'personal',{'body':value,'provenance':'captured'},record)['summary'];second=signed(c,'personal',{'body':value,'provenance':'captured'},record)['summary'];assert first['id']==second['id'] and first['revision']==second['revision']
                card=signed(c,'cards',{'product':'demo','card':dict(value,idempotency_key='shared-'+key)},record)['card'];request('publish','/v1/cards/'+card['id']+'/publish',clients[0]['api'],{'product':'demo'},record=record)
                notes=request('owner','/v1/products/demo/personal',c['api'],record=record)['summaries'];assert any(n['id']==first['id'] for n in notes);assert all(n['body']['idempotency_key'].split('-')[1]==str(c['i']) for n in notes)
                team=request('team','/v1/products/demo/context',c['api'],record=record)['entries'];assert all(set(x['card']['body']['service_ids'])<=({'backend','frontend'} if c['i']==0 else {c['service']}) for x in team)
                request('search','/v1/search',c['api'],{'product':'demo','query':'middleware','limit':8},record=record)
                c['cycles']=c.get('cycles',0)+1
                if args.rare_term_every and c['cycles']%args.rare_term_every==0:
                    try:assert request('search_rare','/v1/search',c['api'],{'product':'demo','query':'absentlexeme','limit':8},record=record)['cards']==[]
                    except AssertionError:pass # a failed status is already counted in errors; keep the load running
                mcp=request('mcp','/mcp/demo',c['api'],{'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'knowledge.search','arguments':{'query':'middleware','limit':8}}},record=record)
                if 'error' in mcp or mcp.get('result',{}).get('isError',False):
                    with lock:errors.append({'operation':'mcp','error':'tool_error'}) # counted, not fatal: an overloaded database must not hide the rest of the run
                request('review','/v1/products/demo/review',clients[0]['api'],record=record)
            def phase(seconds,maximum,prefix,record):
                began=time.monotonic();deadline=began+seconds
                def run(c):
                    n=0
                    while n<maximum and time.monotonic()<deadline:
                        try:cycle(c,prefix+'-'+str(c['i'])+'-'+str(n),record)
                        except Exception:
                            if not args.tolerate_errors:raise
                        n+=1;time.sleep(args.cycle_pause_ms/1000)
                    return n
                with concurrent.futures.ThreadPoolExecutor(max_workers=args.identities) as executor:counts=list(executor.map(run,clients))
                left=deadline-time.monotonic()
                if left>0:time.sleep(left)
                return {'duration_seconds':round(time.monotonic()-began,3),'cycles_per_developer':counts}
            warm=phase(5,10,'warm',False);timings.clear();errors.clear();buckets.clear();origin_time=time.monotonic();steady_wall=time.time();measured_start=metrics()
            if args.statement_stats:sql('SELECT pg_stat_statements_reset()')
            steady=phase(args.duration,args.duration*1000//args.cycle_pause_ms+1,'steady',True)
            statements=json.loads(sql("SELECT COALESCE(json_agg(q),'[]') FROM (SELECT left(regexp_replace(query,'\\s+',' ','g'),400) AS statement,calls,round(total_exec_time::numeric,1) AS total_ms,round(mean_exec_time::numeric,3) AS mean_ms,round(max_exec_time::numeric,1) AS max_ms,rows,shared_blks_hit,shared_blks_read FROM pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) ORDER BY total_exec_time DESC LIMIT 15) q")) if args.statement_stats else None
            events=[] if not args.database_events else [{'minute':None,'line':l[:300]} for l in command('docker','logs',database,check=False).stderr.splitlines() if ('checkpoint' in l or 'automatic' in l) and l[:4]=='2026' or ('checkpoint' in l or 'automatic' in l) and l[:2]=='20']
            table_scans=json.loads(sql("SELECT COALESCE(json_agg(q),'[]') FROM (SELECT relname AS table_name,seq_scan,seq_tup_read,n_live_tup FROM pg_stat_user_tables ORDER BY seq_tup_read DESC LIMIT 6) q")) if args.statement_stats else None
            began=time.monotonic()
            def catchup(c):
                refresh(c);refresh(clients[0])
                for n in range(20):
                    value=body(c,'queued-'+str(c['i'])+'-'+str(n));signed(c,'personal',{'body':value,'provenance':'captured'});card=signed(c,'cards',{'product':'demo','card':dict(value,idempotency_key='shared-'+value['idempotency_key'])})['card'];request('publish','/v1/cards/'+card['id']+'/publish',clients[0]['api'],{'product':'demo'})
            def tolerant_catchup(c):
                try:catchup(c)
                except Exception:
                    if not args.tolerate_errors:raise
            with concurrent.futures.ThreadPoolExecutor(max_workers=args.identities) as executor:list(executor.map(tolerant_catchup,clients))
            burst_seconds=time.monotonic()-began;after=metrics();stop.set();monitor.join(timeout=5);sample()
            assert after['max_nonces_per_device']<1200
            reconnect_missing=0;retention=None;backup_bytes=0;recovery_failure=None
            try:
                # Restart the owned app only; persisted summaries must remain reachable.
                command('docker','restart',application)
                for _ in range(100):
                    try:request('restart_ready','/health/ready',record=False);break
                    except Exception:time.sleep(.1)
                else:raise RuntimeError('Owned application restart failed')
                for c in clients:
                    refresh(c);notes=request('reconnect_owner','/v1/products/demo/personal',c['api'],record=False)['summaries']
                    if not any(n['body']['idempotency_key'].startswith('queued-'+str(c['i'])+'-') for n in notes):
                        if not args.tolerate_errors:raise AssertionError('queued summaries missing after restart')
                        reconnect_missing+=1
                # One actual physical-retention operation after measured growth.
                sql("UPDATE personal_summaries SET expires_at=now()-interval '1 second' WHERE id=(SELECT id FROM personal_summaries ORDER BY created_at LIMIT 1)");retention=json.loads(cli('retention-once'));assert retention['personal']==1
                command('docker','exec',database,'apk','add','--no-cache','age')
                containerwork='/tmp/wk-pilot-backup';command('docker','exec',database,'mkdir','-m','700',containerwork);command('docker','cp',str(args.deploy_scripts),database+':'+containerwork+'/scripts');command('docker','exec',database,'age-keygen','-o',containerwork+'/identity');recipient=command('docker','exec',database,'age-keygen','-y',containerwork+'/identity').stdout.strip();command('docker','exec','-i',database,'sh','-c','umask 077; cat > '+containerwork+'/services',input='[pilot]\nhost=127.0.0.1\nuser=wk_operator\npassword='+operator_password+'\ndbname='+dbname+'\n');command('docker','exec',database,'env','PGSERVICEFILE='+containerwork+'/services','PGSERVICE=pilot','sh',containerwork+'/scripts/backup.sh',recipient,containerwork+'/backup.age');backup_bytes=int(command('docker','exec',database,'stat','-c','%s',containerwork+'/backup.age').stdout)
            except Exception as failure:
                if not args.tolerate_errors:raise
                recovery_failure=type(failure).__name__ # load results above are kept
            latency={}
            for operation,values in timings.items():
                ordered=sorted(values);latency[operation]={'requests':len(values),'p50_ms':round(ordered[(len(ordered)-1)//2],2),'p95_ms':round(ordered[int((len(ordered)-1)*.95)],2),'max_ms':round(max(values),2)}
            resource={}
            for label in ['app','database']:
                rates=[(b[label]['cpu_microseconds']-a[label]['cpu_microseconds'])/(b[label]['observed_at']-a[label]['observed_at'])/10000 for a,b in zip(samples,samples[1:])];resource[label]={'peak_cpu_percent_one_core':round(max(rates),2),'mean_cpu_percent_one_core':round((samples[-1][label]['cpu_microseconds']-samples[0][label]['cpu_microseconds'])/(samples[-1][label]['observed_at']-samples[0][label]['observed_at'])/10000,2),'peak_memory_bytes':max(s[label]['peak_memory_bytes'] for s in samples),'peak_rss_bytes':max(s[label]['process_rss_bytes'] for s in samples),'cpu_cap':2,'throttled_microseconds_delta':samples[-1][label]['throttled_microseconds']-samples[0][label]['throttled_microseconds'],'memory_cap_bytes':(1 if label=='app' else 2)*1024**3}
            result={'platform':{'app':'Linux aarch64 release container, synthetic JWT verifier fixture','database':sql('SELECT version()'),'host':command('uname','-sm').stdout.strip(),'host_cpu_count':os.cpu_count(),'host_memory_bytes':int(command('sysctl','-n','hw.memsize').stdout) if os.uname().sysname=='Darwin' else os.sysconf('SC_PHYS_PAGES')*os.sysconf('SC_PAGE_SIZE'),'docker_vm':command('docker','info','--format','{{.NCPU}} CPUs; {{.MemTotal}} memory bytes').stdout.strip(),'image_id':command('docker','image','inspect','--format','{{.Id}}',args.image).stdout.strip()},'schedule':{'identities':args.identities,'services':['backend','frontend'],'reviewer':'developer0','scoped_admin':'developer1','warmup':warm,'steady':steady,'cycle_operations':10,'rare_term_every_cycles':args.rare_term_every,'tolerate_errors':args.tolerate_errors,'cycle_pause_ms':args.cycle_pause_ms,'payload_bytes':10240,'shared_body_bytes':10247,'offline_catchup':'20 preassembled synthetic summaries per developer; not native disk-queue acceptance','burst_seconds':round(burst_seconds,3),'client':str(args.identities)+' threads, one request each at a time; urllib HTTP and local OpenSSL signing'},'recovery':{'owned_app_restart':recovery_failure is None,'owner_reconnections':args.identities-reconnect_missing,'recovery_failure':recovery_failure,'restore':'separate hosted-restore-drill required'},'requests':sum(len(x) for x in timings.values()),'errors':errors,'latency':latency,'throughput_requests_per_second':round(sum(len(x) for x in timings.values())/(steady['duration_seconds']+burst_seconds),2),'resources':resource,'pool':{'sample_count':len(samples),'peak_connections':max(s['runtime_pool']['connections'] for s in samples),'peak_active':max(s['runtime_pool']['active'] for s in samples),'peak_lock_waits':max(s['runtime_pool']['lock_waits'] for s in samples)},'statement_stats':statements,'table_scans':table_scans,'database_events':events,'steady_started_epoch':round(steady_wall,1),'timeline_per_minute':[{'minute':m,'requests':sum(len(v) for v in b.values()),**{op:{'n':len(b[op]),'p95_ms':round(sorted(b[op])[int((len(b[op])-1)*.95)],1)} for op in ('search','search_rare','collector_cards','collector_personal') if op in b}} for m,b in sorted(buckets.items())],'storage':{'initial':baseline,'after_warmup':measured_start,'after_measured_load':after,'encrypted_snapshot_bytes':backup_bytes,'retention_operation':retention},'limits':{'pool':10,'acquire_ms':5000,'statement_ms':5000,'lock_ms':3000,'http_seconds':30,'inflight':64,'response_bytes':8*1024*1024,'database_max_connections':50,'shared_buffers_bytes':256*1024**2,'work_mem_bytes':4*1024**2},'limitations':['Local Docker VM is not a provisioned remote server or a long-duration capacity test.','Synthetic10KiB technical vocabulary is compressible; measured row density is not a real-content forecast.','No production OIDC/TLS/browser/native capture or model inference exercised by this load fixture.']}
            result['resources']['sampling_note']='Per-container guest uptime captured beside each cgroup read; sampled intervals are approximate. RSS is PID1 only; cgroup memory includes workers/cache. Throttle time counts throttled runnable tasks, not wall-clock outage.'
            result['resources']['actual_container_configuration']={name:json.loads(command('docker','inspect','--format','{\"cpu_nanoseconds\":{{.HostConfig.NanoCpus}},\"memory_bytes\":{{.HostConfig.Memory}},\"readonly\":{{.HostConfig.ReadonlyRootfs}},\"user\":\"{{.Config.User}}\"}',container).stdout) for name,container in [('app',application),('database',database)]}
            args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'output':str(args.output),'requests':result['requests'],'errors':len(errors),'throughput':result['throughput_requests_per_second'],'resource_caps':'app2CPU/1GiB; database2CPU/2GiB','owned_containers_and_network_removed_on_exit':True}))
    finally:
        stop.set()
        for name in reversed(created):command('docker','rm','-f','-v',name,check=False)
        if network_created:command('docker','network','rm',network,check=False)
if __name__=='__main__':main()
