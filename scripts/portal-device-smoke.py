#!/usr/bin/env python3
"""Disposable upstream public-client + signed collector + hosted UI acceptance.
Only wk_test and freshly created synthetic OS-keyring entries are used.
No existing client profiles, hooks or conversation stores are read.
"""
import hashlib,importlib.util,json,os,secrets,shutil,subprocess,tempfile,time,urllib.parse,urllib.request,urllib.error,uuid
from pathlib import Path
OPENSSL=os.environ.get('WK_TEST_OPENSSL') or ('/opt/homebrew/opt/openssl@3/bin/openssl' if Path('/opt/homebrew/opt/openssl@3/bin/openssl').exists() else shutil.which('openssl'))
spec=importlib.util.spec_from_file_location('identity',Path(__file__).with_name('portal-identity-smoke.py'));identity=importlib.util.module_from_spec(spec);spec.loader.exec_module(identity)
class NativeProvider(identity.Provider):
    def do_GET(self):
        parsed=urllib.parse.urlparse(self.path);cfg=self.server.cfg
        if parsed.path=='/.well-known/openid-configuration':
            issuer=cfg['issuer'];return self.send(200,{'issuer':issuer,'authorization_endpoint':issuer+'/authorize','token_endpoint':issuer+'/token','jwks_uri':issuer+'/keys','response_types_supported':['code'],'subject_types_supported':['public'],'id_token_signing_alg_values_supported':['RS256'],'token_endpoint_auth_methods_supported':['none','client_secret_basic'],'code_challenge_methods_supported':['S256']})
        q=urllib.parse.parse_qs(parsed.query)
        if parsed.path!='/authorize' or q.get('client_id')!=['collector-test']:return super().do_GET()
        required=['state','nonce','code_challenge','redirect_uri','response_type','code_challenge_method']
        if any(len(q.get(k,[]))!=1 for k in required):return self.send(400,{})
        redirect=urllib.parse.urlparse(q['redirect_uri'][0])
        if redirect.scheme!='http' or redirect.hostname!='127.0.0.1' or not redirect.port or redirect.path!='/callback' or redirect.query or redirect.fragment or redirect.username or q['response_type']!=['code'] or q['code_challenge_method']!=['S256']:return self.send(400,{})
        code=secrets.token_urlsafe(32);q['synthetic_subject']=[cfg.get('subject','alice')];cfg['codes'][code]=q
        return self.send(302,location=q['redirect_uri'][0]+'?'+urllib.parse.urlencode({'code':code,'state':q['state'][0]}))
    def do_POST(self):
        if self.path!='/token':return super().do_POST()
        cfg=self.server.cfg;n=int(self.headers.get('Content-Length','0'))
        if n>65536:return self.send(400,{})
        raw=self.rfile.read(n);q=urllib.parse.parse_qs(raw.decode());grant=cfg['codes'].get(q.get('code',[''])[0])
        if grant is None:return self.send(400,{})
        if grant.get('client_id')!=['collector-test']:
            # Base provider consumes the same bounded request stream.
            import io
            self.rfile=io.BytesIO(raw);return super().do_POST()
        cfg['codes'].pop(q['code'][0]);subject=grant['synthetic_subject'][0]
        if self.headers.get('Authorization') is not None or q.get('client_id')!=['collector-test'] or q.get('redirect_uri')!=grant['redirect_uri'] or q.get('grant_type')!=['authorization_code'] or identity.b64(hashlib.sha256(q.get('code_verifier',[''])[0].encode()).digest())!=grant['code_challenge'][0]:return self.send(400,{})
        now=int(time.time())
        def token(aud,**extra):
            claims=dict(iss=cfg['issuer'],aud=aud,sub=subject,iat=now,exp=now+300,jti=secrets.token_urlsafe(24),**extra)
            value=identity.b64(json.dumps({'alg':'RS256','kid':'synthetic-portal'}).encode())+'.'+identity.b64(json.dumps(claims).encode());signature=subprocess.check_output([OPENSSL,'dgst','-sha256','-sign',cfg['pem']],input=value.encode(),stderr=subprocess.DEVNULL)
            return value+'.'+identity.b64(signature)
        access=token('wk-collector');cfg.setdefault('issued',{})[subject]=access
        return self.send(200,{'access_token':access,'token_type':'Bearer','expires_in':300,'id_token':token('collector-test',nonce=grant['nonce'][0])})

def acceptance(origin,owner,mutation,command,provider,start):
    assert os.uname().sysname=='Darwin','This native keyring smoke harness currently cleans up macOS synthetic credentials only'
    org=json.loads(owner.request(origin+'/v1/me')[2])['org'];root=origin+'/v1/products/alpha'
    command('provision',{'issuer':provider.cfg['issuer'],'subject':'bob','org':org,'user':'bob','product':'alpha','services':['frontend'],'can_review':True})
    provider.cfg['subject']='bob';bob=identity.Browser();assert bob.request(start(bob))[0]==303;provider.cfg['subject']='alice'
    def devices(browser=owner):
        status,_,raw=browser.request(root+'/devices');assert status==200;return json.loads(raw)['devices']
    assert devices()==[] and devices(bob)==[]
    with tempfile.TemporaryDirectory(prefix='wk-device-live-') as scratch:
        scratch=Path(scratch).resolve();scratch.chmod(0o700);workspace=scratch/'workspace';(workspace/'backend').mkdir(parents=True);consent=scratch/'approval';native=scratch/'native';agent=identity.REPO/'target/debug/wk-agent'
        subprocess.run([str(identity.REPO/'target/debug/wk'),'--state-dir',str(consent),'workspace','approve',str(workspace),'--product','alpha','--service','backend','--yes'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
        binding=consent/(hashlib.sha256(str(workspace).encode()).hexdigest()+'.json');approval=json.loads(binding.read_text())
        def authorize(url):
            browser=identity.Browser();status,headers,_=browser.request(url);assert status==302;assert browser.request(headers['Location'])[0]==200
        proc=None
        try:
            proc=subprocess.Popen([str(agent),'--state-dir',str(native),'device','connect','--binding',str(binding),'--service','backend','--server',origin,'--yes','--no-browser'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            line=proc.stderr.readline();attempt=json.loads(line);assert 'authorization_url' in attempt
            authorize(attempt['authorization_url']);print('Synthetic native sign-in returned; waiting for OS credential storage if prompted.',flush=True)
            stdout,stderr=proc.communicate(timeout=300);assert proc.returncode==0,'Native connect failed: '+stderr[:300];connected=json.loads(stdout);assert connected['connected'] and connected['capture_coverage']=='unknown';native_id=connected['device_id']
            def cli(*args,allowed=True):
                run=subprocess.run([str(agent),'--state-dir',str(native),*args],capture_output=True,text=True,timeout=180)
                if allowed:assert run.returncode==0,run.stderr[:300];return json.loads(run.stdout)
                assert run.returncode!=0;return run
            assert cli('device','check-native-policy')['native_policy_authorized'];cli('device','poll');current=devices()[0];assert current['id']==native_id and current['report']['config_revision']==1 and current['report']['local_owner_sync'] is False
            # Upstream token is returned by this disposable provider only. No keyring extraction.
            alice_token=provider.cfg['issued']['alice']
            def browser(mode):subprocess.run(['node',str(identity.REPO/'web/scripts/device-live.mjs'),origin,native_id,mode],check=True)
            browser('pause');current=devices()[0];assert current['settings']['paused'] and current['revision']==2 and current['report']['config_revision']==1
            # Offline period has no collector poll; the server request stays pending.
            cli('device','poll');assert devices()[0]['report']['config_revision']==2
            browser('ack');browser('resume');cli('device','poll');assert devices()[0]['report']['config_revision']==3
            subprocess.run([str(identity.REPO/'target/debug/wk'),'--state-dir',str(consent),'workspace','pause',str(workspace)],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
            cli('device','poll');assert devices()[0]['report']['local_pause'] is True and devices()[0]['settings']['paused'] is False
            subprocess.run([str(identity.REPO/'target/debug/wk'),'--state-dir',str(consent),'workspace','approve',str(workspace),'--product','alpha','--service','backend','--yes'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
            (native/'device-auth.json').write_text(json.dumps({'expires_at':int(time.time())-10}));cli('device','poll',allowed=False)
            proc=subprocess.Popen([str(agent),'--state-dir',str(native),'device','login','--no-browser'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True);authorize(json.loads(proc.stderr.readline())['authorization_url']);stdout,stderr=proc.communicate(timeout=180);assert proc.returncode==0,stderr[:300];assert json.loads(stdout)['authenticated'];assert json.loads((native/'device-auth.json').read_text())['expires_at']>int(time.time())
            cli('owner-sync','enable','--yes');cli('device','poll');assert devices()[0]['report']['local_owner_sync']
            # Separate synthetic proof key exercises raw protocol failure paths without exposing native key.
            def fixture(subject,service,token_override=None,expected_status=200):
                folder=scratch/(subject+"-"+secrets.token_hex(4));folder.mkdir(mode=0o700);pem=folder/'key.pem';subprocess.run([OPENSSL,'genpkey','-algorithm','ED25519','-out',str(pem)],check=True,stderr=subprocess.DEVNULL);pem.chmod(0o600);public=subprocess.check_output([OPENSSL,'pkey','-in',str(pem),'-pubout','-outform','DER'],stderr=subprocess.DEVNULL)[-32:].hex();installation=str(uuid.uuid4());hash_value=secrets.token_hex(32)
                if token_override is not None:token=token_override
                else:
                    provider.cfg['subject']=subject
                    # Real upstream token exchange uses a fresh public-client PKCE grant.
                    verifier=secrets.token_urlsafe(40);callback='http://127.0.0.1:'+str(identity.allocate())+'/callback';url=provider.cfg['issuer']+'/authorize?'+urllib.parse.urlencode({'client_id':'collector-test','redirect_uri':callback,'response_type':'code','state':secrets.token_urlsafe(32),'nonce':secrets.token_urlsafe(32),'code_challenge':identity.b64(hashlib.sha256(verifier.encode()).digest()),'code_challenge_method':'S256'})
                    client=identity.Browser();status,headers,_=client.request(url);assert status==302;code=urllib.parse.parse_qs(urllib.parse.urlparse(headers['Location']).query)['code'][0]
                    with urllib.request.urlopen(urllib.request.Request(provider.cfg['issuer']+'/token',data=urllib.parse.urlencode({'client_id':'collector-test','grant_type':'authorization_code','code':code,'redirect_uri':callback,'code_verifier':verifier}).encode(),headers={'Content-Type':'application/x-www-form-urlencoded'}),timeout=10) as response:token=json.load(response)['access_token']
                    provider.cfg['subject']='alice'
                registration={'id':installation,'product':'alpha','services':[service],'public_key':public,'approval_hash':hash_value}
                def proof(operation,request=None,register=False):
                    value=registration if register else {'product':'alpha','approval_hash':hash_value,'request':request or {}};body=json.dumps(value,separators=(',',':')).encode();timestamp=int(time.time());nonce=str(uuid.uuid4());path='/collector/'+operation;envelope=json.dumps(['wk-device-v1',origin,'POST',path,timestamp,nonce,installation,hashlib.sha256(token.encode()).hexdigest(),hashlib.sha256(body).hexdigest()],separators=(',',':')).encode();message=folder/'message';message.write_bytes(envelope);message.chmod(0o600);sig=subprocess.check_output([OPENSSL,'pkeyutl','-sign','-rawin','-inkey',str(pem),'-in',str(message)],stderr=subprocess.DEVNULL).hex();headers={'Authorization':'Bearer '+token,'Content-Type':'application/json','X-WK-Device':installation,'X-WK-Time':str(timestamp),'X-WK-Nonce':nonce,'X-WK-Signature':sig}
                    def execute():
                        try:r=urllib.request.urlopen(urllib.request.Request(origin+path,data=body,headers=headers),timeout=10)
                        except urllib.error.HTTPError as e:r=e
                        return r.code,json.loads(r.read())
                    return execute
                assert proof('register',register=True)()[0]==expected_status
                return installation,proof,token
            aid,alice_proof,alice_fixture_token=fixture('alice','backend');bid,bob_proof,bob_token=fixture('bob','frontend');assert all(d['id']!=bid for d in devices());assert all(d['id'] not in (aid,native_id) for d in devices(bob))
            for token in [alice_token,bob_token]:
                assert identity.Browser().request(origin+'/v1/me',headers={'Authorization':'Bearer '+token})[0]==401
                assert identity.Browser().request(origin+'/mcp/alpha',headers={'Authorization':'Bearer '+token})[0]==401
            replay=alice_proof('policy');assert replay()[0]==200 and replay()[0]==401
            body={'schema_version':1,'idempotency_key':'synthetic-device-owner','policy_revision':1,'title':'Synthetic device summary','intent':'Bound request work','rationale':'Scoped middleware','decisions':[],'constraints':[],'open_questions':[],'service_ids':['backend'],'evidence':[],'lifecycle':'proposed','capture_completeness':'complete','base_revision':None};personal={'body':body,'provenance':'captured'}
            assert alice_proof('personal',personal)()[0]==403
            d=next(d for d in devices() if d['id']==aid);assert owner.request(root+'/devices/'+aid+'/settings',{'expected_revision':d['revision'],'settings':{'paused':False,'owner_sync':True,'activity':False}},mutation,'PATCH')[0]==200
            assert alice_proof('personal',personal)()[0]==200
            assert bob.request(root+'/personal')[0]==200 and json.loads(bob.request(root+'/personal')[2])['summaries']==[]
            assert bob_proof('personal',personal)()[0]==403
            assert alice_proof('personal',{'body':dict(body,idempotency_key='wider-scope',service_ids=['backend','frontend']),'provenance':'captured'})()[0]==403
            browser('revoke');assert cli('device','poll',allowed=False).returncode!=0
            d=next(d for d in devices() if d['id']==aid);assert owner.request(root+'/devices/'+aid+'/revoke',{'expected_revision':d['revision']},mutation)[0]==200
            assert alice_proof('personal',dict(personal,body=dict(body,idempotency_key='after-revoke')))()[0]==401
            assert alice_proof('register',register=True)()[0]==403
            fixture('alice','backend',token_override=alice_fixture_token,expected_status=403)
            fixture('alice','backend')
            print('Real native sign-in, signed controls and two-identity browser acceptance passed.',flush=True)
        finally:
            if proc is not None and proc.poll() is None:proc.terminate();proc.wait(timeout=10)
            # Delete only accounts for this freshly allocated synthetic state directory.
            if os.uname().sysname=='Darwin':
                account=hashlib.sha256(str(native).encode()).hexdigest()
                for purpose in ['capture-key','ipc-key','device-key','server-token']:subprocess.run(['/usr/bin/security','delete-generic-password','-s','workspace-knowledge','-a',account+':'+purpose],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    return {'native_pkce_and_os_keyring':True,'native_reauthentication_after_expiry':True,'remote_resume_preserves_local_pause':True,'requested_applied_pause_resume':True,'owner_opt_in_and_scope':True,'two_identity_device_privacy':True,'revocation_non_resurrection':True,'revoked_credential_replacement_denied':True,'renewed_credential_replacement_allowed':True,'signed_native_policy':True,'replay_and_api_mcp_role_escape':True,'real_browser_devices':True,'system_browser_launch_verified':False}
if __name__=='__main__':identity.main(acceptance,NativeProvider,collector=True)
