#!/usr/bin/env python3
"""Real disposable upstream PKCE/browser and typed OIDC administration acceptance."""
import importlib.util,json,os,secrets,subprocess,time
from pathlib import Path
spec=importlib.util.spec_from_file_location('identity',Path(__file__).with_name('portal-identity-smoke.py'));identity=importlib.util.module_from_spec(spec);spec.loader.exec_module(identity)
class AdminProvider(identity.Provider):
    def send(self,status,body=None,location=None):
        if status==200 and isinstance(body,dict) and 'id_token' in body:
            import base64
            encoded=body['id_token'].split('.')[1];claims=json.loads(base64.urlsafe_b64decode(encoded+'='*(-len(encoded)%4)));claims['aud']='wk-api';claims.pop('nonce',None);claims['jti']=secrets.token_urlsafe(24)
            raw=identity.b64(json.dumps({'alg':'RS256','kid':'synthetic-portal'}).encode())+'.'+identity.b64(json.dumps(claims).encode());signed=subprocess.check_output(['/usr/bin/openssl','dgst','-sha256','-sign',self.server.cfg['pem']],input=raw.encode(),stderr=subprocess.DEVNULL);body=dict(body,access_token=raw+'.'+identity.b64(signed));self.server.cfg.setdefault('issued',{})[claims['sub']]=body['access_token']
        return super().send(status,body,location)
def acceptance(origin,owner,mutation,command,provider,start):
    org=json.loads(owner.request(origin+'/v1/me')[2])['org'];root=origin+'/v1/products/alpha'
    command('provision',{'issuer':provider.cfg['issuer'],'subject':'bob','org':org,'user':'bob','product':'alpha','services':['backend'],'can_review':True})
    provider.cfg['subject']='bob';bob=identity.Browser();assert bob.request(start(bob))[0]==303;provider.cfg['subject']='alice'
    env=dict(os.environ,WK_DATABASE_URL=os.environ['WK_TEST_DATABASE_URL'],WK_ADMIN_TEST_ISSUER=provider.cfg['issuer'],WK_ADMIN_TEST_AUDIENCE='wk-api',WK_ADMIN_TEST_JWKS_URL=provider.cfg['issuer']+'/keys')
    def cli(subject,request,product='alpha',allowed=True,token=None):
        token=token or provider.cfg['issued'][subject];data={'token':token,'product':product,'command':request};run=subprocess.run([str(identity.REPO/'target/debug/wk-server'),'--dev-admin'],env=env,input=json.dumps(data),text=True,capture_output=True,timeout=30)
        assert token not in run.stdout and token not in run.stderr
        if allowed:assert run.returncode==0,run.stderr[:200];return json.loads(run.stdout)
        assert run.returncode!=0;return run
    cli('bob',{'kind':'members'},allowed=False);cli('alice',{'kind':'members'},allowed=False)
    command('admin-assign',{'org':org,'user':'alice','product':'alpha','services':['backend']})
    assert {m['user'] for m in cli('alice',{'kind':'members'})['members']}=={'alice','bob'}
    cli('alice',{'kind':'members'},product='beta',allowed=False)
    cli('alice',{'kind':'members'},token='invalid-synthetic-token',allowed=False)
    cli('alice',{'kind':'member-put','member':{'user':'bob','services':['frontend'],'can_review':True}},allowed=False)
    cli('alice',{'kind':'member-put','member':{'user':'unmapped','services':['backend'],'can_review':False}},allowed=False)
    policy={'private_days':1,'report_days':1,'allow_loopback':True,'https_origins':[]};cli('alice',{'kind':'policy-put','policy':policy});assert json.loads(owner.request(origin+'/v1/policy?product=alpha')[2])['policy']['restrictions']==policy
    cli('alice',{'kind':'policy-put','policy':dict(policy,private_days=30)},allowed=False)
    revision=json.loads(owner.request(origin+'/v1/policy?product=alpha')[2])['policy']['policy_revision'];body={'schema_version':1,'idempotency_key':'admin-private-synthetic','policy_revision':revision,'title':'Synthetic private summary','intent':'Bound work','rationale':'Scoped middleware','decisions':[],'constraints':[],'open_questions':[],'service_ids':['backend'],'evidence':[],'lifecycle':'proposed','capture_completeness':'complete','base_revision':None}
    status,_,raw=bob.request(root+'/personal',{'body':body,'provenance':'captured'},{'Origin':origin,'Content-Type':'application/json','X-CSRF-Token':json.loads(bob.request(origin+'/v1/me')[2])['csrf_token']});assert status==200;note=json.loads(raw)['summary'];assert abs(note['expires_at_ms']-note['created_at_ms']-86400000)<1000
    assert json.loads(owner.request(root+'/personal')[2])['summaries']==[]
    cli('alice',{'kind':'member-remove','user':'bob'});assert json.loads(bob.request(origin+'/v1/me')[2])['products']==[]
    cli('bob',{'kind':'members'},allowed=False)
    return {'real_upstream_admin_oidc':True,'reviewer_not_admin':True,'product_and_service_admin_scope':True,'admin_private_summary_denied':True,'policy_narrowing_and_creation_relative_retention':True,'membership_remove_enforced':True,'admin_token_not_echoed':True}
if __name__=='__main__':identity.main(acceptance,AdminProvider)
