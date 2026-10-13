#!/usr/bin/env python3
"""Real loopback cookie/CSRF and owner workflow with a synthetic OIDC provider."""
import importlib.util,json,subprocess
from pathlib import Path
spec=importlib.util.spec_from_file_location('identity',Path(__file__).with_name('portal-identity-smoke.py'));identity=importlib.util.module_from_spec(spec);spec.loader.exec_module(identity)
def acceptance(origin,owner,mutation,command,provider,start):
    me=json.loads(owner.request(origin+'/v1/me')[2]);org=me['org'];root=origin+'/v1/products/alpha/personal'
    command('provision',{'issuer':provider.cfg['issuer'],'subject':'bob','org':org,'user':'bob','product':'alpha','services':['backend'],'can_review':True})
    provider.cfg['subject']='bob';bob=identity.Browser();assert bob.request(start(bob))[0]==303;provider.cfg['subject']='alice'
    body={'schema_version':1,'idempotency_key':'live-owner','policy_revision':1,'title':'Tenant limiter','intent':'Bound request work','rationale':'Shared middleware','decisions':['Per tenant limits'],'constraints':[],'open_questions':[],'service_ids':['backend'],'evidence':[],'lifecycle':'proposed','capture_completeness':'complete','base_revision':None};payload={'body':body,'provenance':'manual'}
    assert owner.request(root,payload,dict(mutation,Origin='https://evil.example'))[0]==403
    assert owner.request(root,payload,{'Origin':origin,'Content-Type':'application/json'})[0]==403
    status,_,data=owner.request(root,payload,mutation);assert status==200;note=json.loads(data)['summary'];path=root+'/'+note['id']
    retry=json.loads(owner.request(root,payload,mutation)[2])['summary'];assert retry['id']==note['id'] and retry['expires_at_ms']==note['expires_at_ms']
    assert bob.request(path)[0]==404 and json.loads(bob.request(root)[2])['summaries']==[]
    assert json.loads(bob.request(origin+'/v1/products/alpha/review')[2])['cards']==[]
    assert json.loads(owner.request(origin+'/v1/search',{'product':'alpha','query':'Tenant limiter','limit':20},mutation)[2])['cards']==[]
    canary='sk-proj-SYNTHETIC_PRIVATE_CANARY_012345678901234567890';bad={'body':dict(body,idempotency_key='canary',intent=canary),'provenance':'manual'};status,_,data=owner.request(root,bad,mutation);assert status==422 and canary.encode() not in data
    status,_,data=owner.request(path,{'expected_revision':1,'body':dict(body,intent='Bound updated work')},mutation,method='PATCH');assert status==200;updated=json.loads(data)['summary'];assert updated['revision']==2 and updated['expires_at_ms']==note['expires_at_ms']
    assert owner.request(path,{'expected_revision':1,'body':body},mutation,method='PATCH')[0]==409
    status,_,data=owner.request(path+'/submit',{'expected_revision':2},mutation);assert status==200;card=json.loads(data)['card'];assert not card['published'];assert json.loads(owner.request(path+'/submit',{'expected_revision':2},mutation)[2])['card']['id']==card['id']
    queue=json.loads(bob.request(origin+'/v1/products/alpha/review')[2])['cards'];assert len(queue)==1 and queue[0]['id']==card['id']
    assert owner.request(path,{'expected_revision':2},mutation,method='DELETE')[0]==200;assert owner.request(path)[0]==404;assert owner.request(root,payload,mutation)[0]==409;assert bob.request(path)[0]==404
    subprocess.run(['node',str(identity.REPO/'web/scripts/owner-live.mjs'),origin],check=True)
    return {'owner_cookie_csrf':True,'owner_only_and_safe_errors':True,'fixed_expiry_retry_revision':True,'explicit_unpublished_review_copy':True,'deletion_retry_guard':True,'real_browser_owner_workflow':True}
if __name__=='__main__':identity.main(acceptance)
