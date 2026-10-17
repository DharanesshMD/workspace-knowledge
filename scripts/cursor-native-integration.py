"""Synthetic native capture with real keyring and explicit loopback inference.
Requires WK_TEST_DATABASE_URL=/wk_test. Never reads existing developer transcripts.
"""
import argparse, hashlib, json, os, re, secrets, shutil, socket, subprocess, tempfile, time, urllib.request, urllib.error
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def request(url,token,body=None):
    headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'}
    req=urllib.request.Request(url,data=None if body is None else json.dumps(body).encode(),headers=headers)
    with urllib.request.urlopen(req,timeout=10) as r:return json.load(r)
def free_port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--inference',required=True);ap.add_argument('--model',required=True);ap.add_argument('--bin-dir',type=Path,default=ROOT/'target/debug');ap.add_argument('--output',type=Path,required=True);args=ap.parse_args()
    db=os.environ['WK_TEST_DATABASE_URL'];assert db.split('?')[0].endswith('/wk_test')
    from urllib.parse import urlparse
    endpoint=urlparse(args.inference);assert endpoint.scheme=='http' and endpoint.hostname in ('127.0.0.1','::1')
    assert 'cloud' not in args.model.lower(), 'explicitly local model required'
    scratch=Path(tempfile.mkdtemp(prefix='wk-native-test-')).resolve();scratch.chmod(0o700)
    workspace=scratch/'workspace';(workspace/'backend').mkdir(parents=True);(workspace/'backend/retry.ts').write_text('export const retries = 3;\n')
    consent=scratch/'consent';state=scratch/'agent';consent.mkdir(mode=0o700);state.mkdir(mode=0o700)
    token=secrets.token_urlsafe(32);token_b=secrets.token_urlsafe(32);env=dict(os.environ,WK_DATABASE_URL=db,WK_DEV_TOKEN=token,WK_DEV_TOKEN_B=token_b,WK_PORT='0',WK_ACCESS_TOKEN=token)
    server=subprocess.Popen([str(args.bin_dir/'wk-server'),'--dev'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
    daemon=None;source_root=None
    def cli(binary,*parts,input=None,timeout=180):
        r=subprocess.run([str(args.bin_dir/binary),*map(str,parts)],input=input,text=True,capture_output=True,env=env,timeout=timeout)
        if r.returncode:raise RuntimeError(f'{binary} {parts[0] if parts else ""} failed: '+r.stderr[:500])
        return json.loads(r.stdout) if r.stdout.strip() else {}
    def agent(*parts,**kw):return cli('wk-agent','--state-dir',state,*parts,**kw)
    try:
        line=server.stderr.readline();assert 'http://127.0.0.1:' in line,line
        base='http://'+line.split('http://')[1].split()[0]
        cli('wk','--state-dir',consent,'workspace','approve',workspace,'--product','demo','--service','backend','--yes')
        binding=consent/(hashlib.sha256(str(workspace).encode()).hexdigest()+'.json')
        port=free_port()
        agent('init','--binding',binding,'--service','backend','--server',base,'--summarizer','local','--inference',args.inference,'--model',args.model,'--port',port,'--yes')
        agent('cursor-enroll','--yes')
        slug=re.sub('[^A-Za-z0-9]','-',str(workspace).lstrip('/'))
        source_root=Path.home()/'.cursor/projects'/slug
        source=source_root/'agent-transcripts'/'synthetic-session'/'synthetic-session.jsonl';source.parent.mkdir(parents=True)
        records=[{'role':'user','message':{'content':[{'type':'text','text':'For this software component, retries require an idempotency key to avoid duplicate operations. Keep retries bounded to three.'}]}},{'role':'assistant','message':{'content':[{'type':'tool_use','name':'Read','input':{'path':str(workspace/'backend/retry.ts')}},{'type':'text','text':'The decision is to require idempotency keys and limit retries to three; implementation still needs review.'}]}},{'type':'turn_ended','status':'success'}]
        source.write_text(''.join(json.dumps(v)+'\n' for v in records))
        def start():
            proc=subprocess.Popen([str(args.bin_dir/'wk-agent'),'--state-dir',str(state),'serve'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            for _ in range(80):
                with socket.socket() as sock:
                    if sock.connect_ex(('127.0.0.1',port))==0:return proc
                if proc.poll() is not None:raise RuntimeError('collector failed to start')
                time.sleep(.1)
            raise RuntimeError('collector did not listen')
        def stop(proc):
            proc.terminate();proc.wait(timeout=10)
        payload={'hook_event_name':'stop','cursor_version':'3.23.23','workspace_roots':[str(workspace)],'conversation_id':'synthetic-session','generation_id':'generation-1','status':'completed','transcript_path':str(source)}
        daemon=start();agent('hook','--client','cursor','--event','stop',input=json.dumps(payload));stop(daemon);daemon=None
        agent('process');drafts=agent('drafts')['private_drafts'];assert len(drafts)==1,agent('doctor')
        first=drafts[0];card=first['card'];assert 'idempotency' in json.dumps(card).lower(), 'model lost the decision'
        assert card['capture_completeness']=='complete';assert card['lifecycle']=='proposed';assert str(source) not in json.dumps(card)
        assert any(e['reference']=='backend/retry.ts' for e in card['evidence'])
        args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(card,indent=2)+'\n');args.output.chmod(0o600)
        # Crash/replay-equivalent restart: callback and unchanged source cannot make another draft.
        daemon=start();agent('hook','--client','cursor','--event','stop',input=json.dumps(payload));stop(daemon);daemon=None
        agent('process');assert len(agent('drafts')['private_drafts'])==1
        submitted=agent('submit','--id',first['id'],'--reviewed')['card'];cid=submitted['id']
        request(base+'/v1/cards/'+cid+'/publish',token,{'product':'demo'})
        found=request(base+'/v1/search',token_b,{'product':'demo','query':'idempotency','limit':20})
        assert cid in [c['id'] for c in found['cards']]
        rpc = urllib.request.Request(base+'/mcp/demo', data=json.dumps({'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'knowledge.search','arguments':{'query':'idempotency','limit':20}}}).encode(), headers={'Authorization':'Bearer '+token_b,'Content-Type':'application/json','Accept':'application/json, text/event-stream','MCP-Protocol-Version':'2025-03-26'})
        with urllib.request.urlopen(rpc,timeout=10) as response:
            assert cid in json.dumps(json.load(response)), 'teammate MCP retrieval lost native card'
        notices=request(base+'/v1/products/demo/overlaps',token_b,{'files':['backend/retry.ts'],'limit':20})['notices']
        assert any(n['card_id']==cid and n['revision']==1 for n in notices)
        # A later safe turn is recovered without any new callback.
        with source.open('a') as f:f.write(''.join(json.dumps(v)+'\n' for v in records))
        agent('process');later=agent('drafts')['private_drafts'];assert len(later)==1
        assert later[0]['id']!=first['id']
        # Sensitive continuation invalidates the pending candidate before submission.
        with source.open('a') as f:f.write(json.dumps({'role':'user','message':{'content':[{'type':'text','text':'password = OWNER-RAW-CANARY'}]}})+'\n')
        agent('process');doctor=agent('doctor');assert doctor['native_cursor']['sessions'].get('held')==1
        assert 'OWNER-RAW-CANARY' not in json.dumps(request(base+'/v1/search',token_b,{'product':'demo','query':'idempotency','limit':20}))
        rejected=subprocess.run([str(args.bin_dir/'wk-agent'),'--state-dir',str(state),'submit','--id',later[0]['id'],'--reviewed'],env=env,capture_output=True,text=True)
        assert rejected.returncode!=0 and 'OWNER-RAW-CANARY' not in rejected.stderr
        try:request(base+'/v1/search','invalid',{'product':'demo','query':'idempotency','limit':5});raise AssertionError('unauthorized access')
        except urllib.error.HTTPError as e:assert e.code==401
        print(json.dumps({'passed':True,'model':args.model,'real_keyring':True,'capture':'synthetic callback and transcript','reviewed_card':str(args.output),'replay_deduplicated':True,'sensitive_source_held':True,'team_retrieval':True,'teammate_mcp_retrieval':True}))
    finally:
        if daemon is not None:daemon.terminate();daemon.wait(timeout=10)
        server.terminate();server.wait(timeout=10)
        # Remove only this test's native credentials and synthetic source.
        for purpose in ('capture-key','ipc-key','server-token'):
            account=hashlib.sha256(str(state).encode()).hexdigest()+':'+purpose
            subprocess.run(['security','delete-generic-password','-s','workspace-knowledge','-a',account],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        if source_root and source_root.exists():shutil.rmtree(source_root)
        shutil.rmtree(scratch)
if __name__=='__main__':main()
