#!/usr/bin/env python3
"""Disposable loopback OIDC acceptance; no real identities or conversation access."""
import argparse, base64, hashlib, http.cookiejar, http.server, json, os, secrets, socket, subprocess, tempfile, threading, time, urllib.parse, urllib.request, urllib.error
from pathlib import Path
REPO=Path(__file__).resolve().parents[1]
def b64(value):return base64.urlsafe_b64encode(value).rstrip(b'=').decode()
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*args):return None
class Browser:
    def __init__(self):
        self.jar=http.cookiejar.CookieJar();self.client=urllib.request.build_opener(NoRedirect(),urllib.request.HTTPCookieProcessor(self.jar))
    def request(self,url,body=None,headers=None,method=None):
        request=urllib.request.Request(url,data=None if body is None else json.dumps(body).encode(),headers=headers or {},method=method)
        try:r=self.client.open(request,timeout=15)
        except urllib.error.HTTPError as e:r=e
        return r.code,r.headers,r.read(262145)
class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def send(self,status,body=None,location=None):
        data=json.dumps(body).encode() if body is not None else b'';self.send_response(status);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)))
        if location:self.send_header('Location',location)
        self.end_headers();self.wfile.write(data)
    def do_GET(self):
        parsed=urllib.parse.urlparse(self.path);cfg=self.server.cfg;issuer=cfg['issuer']
        if parsed.path=='/.well-known/openid-configuration':
            return self.send(200,{'issuer':issuer,'authorization_endpoint':issuer+'/authorize','token_endpoint':issuer+'/token','jwks_uri':issuer+'/keys','response_types_supported':['code'],'subject_types_supported':['public'],'id_token_signing_alg_values_supported':['RS256'],'token_endpoint_auth_methods_supported':['none'] if cfg['secret'] is None else ['client_secret_basic'],'code_challenge_methods_supported':['S256']})
        if parsed.path=='/keys':return self.send(200,{'keys':[{'kty':'RSA','kid':'synthetic-portal','alg':'RS256','use':'sig','n':cfg['key']['n'],'e':cfg['key']['e']}]})
        if parsed.path=='/authorize':
            q=urllib.parse.parse_qs(parsed.query);required=['state','nonce','code_challenge','redirect_uri','client_id','response_type','code_challenge_method']
            if any(len(q.get(k,[]))!=1 for k in required) or q['client_id'][0]!='portal-test' or q['redirect_uri'][0]!=cfg['callback'] or q['response_type'][0]!='code' or q['code_challenge_method'][0]!='S256':return self.send(400,{'error':'invalid_request'})
            code=secrets.token_urlsafe(32);q['synthetic_subject']=[cfg.get('subject','alice')];cfg['codes'][code]=q
            return self.send(302,location=cfg['callback']+'?'+urllib.parse.urlencode({'code':code,'state':q['state'][0]}))
        self.send(404,{})
    def do_POST(self):
        cfg=self.server.cfg
        if self.path!='/token':return self.send(404,{})
        n=int(self.headers.get('Content-Length','0'))
        if n>65536:return self.send(400,{})
        q=urllib.parse.parse_qs(self.rfile.read(n).decode());code=q.get('code',[''])[0];grant=cfg['codes'].pop(code,None)
        expected=None if cfg['secret'] is None else 'Basic '+base64.b64encode(('portal-test:'+cfg['secret']).encode()).decode()
        public_ok=cfg['secret'] is not None or q.get('client_id',[''])[0]=='portal-test'
        if not public_ok or grant is None or self.headers.get('Authorization')!=expected or q.get('redirect_uri',[''])[0]!=cfg['callback'] or q.get('grant_type',[''])[0]!='authorization_code' or b64(hashlib.sha256(q.get('code_verifier',[''])[0].encode()).digest())!=grant['code_challenge'][0]:return self.send(400,{'error':'invalid_grant'})
        nonce=grant['nonce'][0]
        if cfg.pop('bad_nonce_once',False):nonce='wrong-nonce'
        now=int(time.time());claims={'iss':cfg['issuer'],'aud':'portal-test','sub':grant['synthetic_subject'][0],'iat':now,'exp':now+300,'nonce':nonce}
        access=secrets.token_urlsafe(32)
        if cfg.pop('good_hash_once',False):claims['at_hash']=b64(hashlib.sha256(access.encode()).digest()[:16])
        for flag,field,value in [('bad_issuer_once','iss','https://evil.example'),('bad_audience_once','aud','other-client'),('expired_once','exp',0),('bad_hash_once','at_hash',b64(hashlib.sha256(b'wrong-synthetic-token').digest()[:16]))]:
            if cfg.pop(flag,False):claims[field]=value
        raw=b64(json.dumps({'alg':'RS256','kid':'synthetic-portal'}).encode())+'.'+b64(json.dumps(claims).encode())
        signed=subprocess.check_output(['/usr/bin/openssl','dgst','-sha256','-sign',cfg['pem']],input=raw.encode(),stderr=subprocess.DEVNULL)
        if cfg.pop('bad_signature_once',False):signed=bytes([signed[0]^1])+signed[1:]
        self.send(200,{'access_token':access,'token_type':'Bearer','expires_in':300,'id_token':raw+'.'+b64(signed)})
def allocate():
    with socket.socket() as sock:sock.bind(('127.0.0.1',0));return sock.getsockname()[1]
def main(owner_acceptance=None,provider_handler=Provider,collector=False):
    parser=argparse.ArgumentParser();parser.add_argument('--serve',action='store_true');parser.add_argument('--public-client',action='store_true');args=parser.parse_args()
    db=os.environ['WK_TEST_DATABASE_URL'];assert db.endswith('/wk_test'),'Only disposable wk_test accepted'
    exe=REPO/'target/debug/wk-server';assert exe.exists()
    results={};server=None;provider=None
    with tempfile.TemporaryDirectory(prefix='wk-portal-identity-') as private:
        os.chmod(private,0o700);key=json.loads((REPO/'crates/server/tests/fixtures/synthetic-rsa-a.json').read_text());pem=Path(private)/'synthetic.pem';pem.write_text(key['private_pkcs8_pem']);pem.chmod(0o600)
        port=allocate();origin=f'http://127.0.0.1:{port}';provider=http.server.ThreadingHTTPServer(('127.0.0.1',0),provider_handler);issuer=f'http://127.0.0.1:{provider.server_port}'
        provider.cfg={'issuer':issuer,'callback':origin+'/auth/callback','key':key,'pem':str(pem),'secret':None if args.public_client else secrets.token_urlsafe(32),'codes':{}};threading.Thread(target=provider.serve_forever,daemon=True).start()
        env=dict(os.environ,WK_DATABASE_URL=db,WK_PORT=str(port),WK_DEV_TOKEN=secrets.token_urlsafe(32),WK_CONSOLE_ORIGIN=origin,WK_WEB_DIR=str(REPO/'web/dist'),WK_BROWSER_CLIENT_ID='portal-test',WK_BROWSER_CLIENT_SECRET=provider.cfg['secret'] or '',WK_BROWSER_TEST_ISSUER=issuer)
        if collector:
            env.update(WK_COLLECTOR_TEST_ISSUER=issuer,WK_COLLECTOR_TEST_AUDIENCE='wk-collector',WK_COLLECTOR_TEST_JWKS_URL=issuer+'/keys',WK_COLLECTOR_TEST_CLIENT_ID='collector-test')
        for name in ['WK_OIDC_ISSUER','WK_OIDC_AUDIENCE','WK_OIDC_JWKS_URL']:env.pop(name,None)
        def command(mode,data=None):
            subprocess.run([str(exe),mode],input=None if data is None else json.dumps(data).encode(),env=env,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
        command('migrate');org='portal-'+secrets.token_hex(8)
        def grant(product):return {'issuer':issuer,'subject':'alice','org':org,'user':'alice','product':product,'services':['backend'],'can_review':product=='beta'}
        command('provision',grant('alpha'));command('provision',grant('beta'))
        try:
            server=subprocess.Popen([str(exe),'--dev'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
            ready=server.stderr.readline();assert 'API listening' in ready,'Server did not initialize'
            def start(browser):
                status,headers,_=browser.request(origin+'/auth/login');assert status==303;status,headers,_=browser.request(headers['Location']);assert status==302;return headers['Location']
            owner=Browser();assert owner.request(origin+'/v1/me')[0]==401
            callback=start(owner);assert Browser().request(callback)[0]==401
            assert owner.request(callback)[0]==303;assert owner.request(callback)[0]==401;results['browser_binding_and_replay']=True
            status,headers,data=owner.request(origin+'/v1/me');assert status==200 and headers['Cache-Control']=='no-store';me=json.loads(data);assert [p['product'] for p in me['products']]==['alpha','beta'];assert 'access_token' not in me and 'id_token' not in me;results['scoped_products']=True
            assert owner.request(origin+'/v1/policy?product=alpha')[0]==200
            assert owner.request(origin+'/v1/policy?product=unassigned')[0] in (403,404);results['product_api_scope']=True
            assert owner.request(origin+'/mcp/alpha')[0]==401;results['mcp_cookie_rejected']=True
            mutation={'Origin':origin,'Content-Type':'application/json','X-CSRF-Token':me['csrf_token']}
            assert owner.request(origin+'/auth/logout',{},dict(mutation,Origin='https://evil.example'))[0]==403
            assert owner.request(origin+'/auth/logout',{}, {'Origin':origin,'Content-Type':'application/json'})[0]==403;results['csrf_and_origin']=True
            for flag in ['bad_nonce_once','bad_signature_once','bad_issuer_once','bad_audience_once','expired_once','bad_hash_once']:
                provider.cfg[flag]=True;other=Browser();assert other.request(start(other))[0]==401;assert other.request(origin+'/v1/me')[0]==401;results[flag.replace('_once','_rejected')]=True
            if provider_handler is Provider:
                provider.cfg['good_hash_once']=True;hashed=Browser();assert hashed.request(start(hashed))[0]==303;assert hashed.request(origin+'/v1/me')[0]==200;results['valid_access_hash']=True
            if owner_acceptance:results.update(owner_acceptance(origin,owner,mutation,command,provider,start))
            command('revoke',grant('alpha'));me=json.loads(owner.request(origin+'/v1/me')[2]);assert [p['product'] for p in me['products']]==['beta'];results['membership_revocation']=True
            assert owner.request(origin+'/v1/policy?product=alpha')[0] in (403,404)
            session_cookie='; '.join(c.name+'='+c.value for c in owner.jar if c.name=='wk_dev_session')
            assert session_cookie
            assert owner.request(origin+'/auth/logout',{},mutation)[0]==200;assert Browser().request(origin+'/v1/me',headers={'Cookie':session_cookie})[0]==401;assert owner.request(origin+'/v1/me')[0]==401;results['logout']=True
            print(json.dumps({'passed':results,'origin':origin,'provider':issuer,'production_provider_verified':False}),flush=True)
            if args.serve:
                print('Synthetic portal running for browser verification; press Enter to stop.',flush=True)
                input()
        finally:
            if server is not None:server.terminate();server.wait(timeout=10)
            provider.shutdown();provider.server_close()
if __name__=='__main__':main()
