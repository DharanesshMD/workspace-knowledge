#!/usr/bin/env python3
"""Real two-identity team feed acceptance; local test provider and disposable DB only."""
import importlib.util,json,subprocess
from pathlib import Path
spec=importlib.util.spec_from_file_location('identity',Path(__file__).with_name('portal-identity-smoke.py'));identity=importlib.util.module_from_spec(spec);spec.loader.exec_module(identity)
def acceptance(origin,owner,mutation,command,provider,start):
    org=json.loads(owner.request(origin+'/v1/me')[2])['org']
    command('provision',{'issuer':provider.cfg['issuer'],'subject':'bob','org':org,'user':'bob','product':'alpha','services':['backend','frontend'],'can_review':True})
    provider.cfg['subject']='bob';bob=identity.Browser();assert bob.request(start(bob))[0]==303;provider.cfg['subject']='alice';me=json.loads(bob.request(origin+'/v1/me')[2]);headers={'Origin':origin,'Content-Type':'application/json','X-CSRF-Token':me['csrf_token']};root=origin+'/v1/products/alpha'
    body={'schema_version':1,'idempotency_key':'team-live','policy_revision':1,'title':'Synthetic team limiter','intent':'Bound shared request work','rationale':'Shared middleware','decisions':['Per tenant limits'],'constraints':[],'open_questions':[],'service_ids':['backend'],'evidence':[{'kind':'file_observation','reference':'backend/limiter.rs'}],'lifecycle':'proposed','capture_completeness':'complete','base_revision':None}
    private=dict(body,idempotency_key='owner-unshared',evidence=[],title='Unshared owner summary');assert owner.request(root+'/personal',{'body':private,'provenance':'manual'},mutation)[0]==200
    status,_,data=bob.request(origin+'/v1/cards',{'product':'alpha','card':body},headers);assert status==200;card=json.loads(data)['card'];assert json.loads(bob.request(root+'/context')[2])['entries']==[]
    assert bob.request(origin+'/v1/cards/'+card['id']+'/publish',{'product':'alpha'},headers)[0]==200
    mixed=dict(body,idempotency_key='mixed-team',title='Mixed service summary',service_ids=['backend','frontend']);status,_,data=bob.request(origin+'/v1/cards',{'product':'alpha','card':mixed},headers);assert status==200;mixed_id=json.loads(data)['card']['id'];assert bob.request(origin+'/v1/cards/'+mixed_id+'/publish',{'product':'alpha'},headers)[0]==200
    feed=json.loads(owner.request(root+'/context')[2])['entries'];assert len(feed)==1 and feed[0]['card']['id']==card['id'] and feed[0]['author']=='bob';assert len(json.loads(bob.request(root+'/context')[2])['entries'])==2
    assert json.loads(owner.request(root+'/context?author=unassigned')[2])['entries']==[];assert json.loads(owner.request(root+'/context?before='+mixed_id)[2])['entries']==[]
    revised=dict(body,idempotency_key='revised-team',base_revision=1,intent='Bound revised shared request work');status,_,data=bob.request(origin+'/v1/cards/'+card['id']+'/correct',{'product':'alpha','card':revised},headers);assert status==200;assert json.loads(owner.request(root+'/context')[2])['entries']==[];assert bob.request(origin+'/v1/cards/'+card['id']+'/publish',{'product':'alpha'},headers)[0]==200;feed=json.loads(owner.request(root+'/context')[2])['entries'];assert feed[0]['card']['revision']==2
    activity={'session_id':'synthetic-shared-session','services':['backend'],'files':['backend/limiter.rs'],'intent':'Review bounded shared work','ended':False};assert bob.request(root+'/activity',activity,headers)[0]==200;assert bob.request(root+'/activity',dict(activity,session_id='synthetic-mixed-session',services=['backend','frontend']),headers)[0]==200;reports=json.loads(owner.request(root+'/activity')[2])['activity'];assert len(reports)==1 and 'session_id' not in reports[0]
    subprocess.run(['node',str(identity.REPO/'web/scripts/team-live.mjs'),origin],check=True)
    assert bob.request(root+'/activity',dict(activity,ended=True),headers)[0]==200;assert json.loads(owner.request(root+'/activity')[2])['activity']==[]
    assert bob.request(origin+'/v1/cards/'+card['id']+'/withdraw',{'product':'alpha','revision':2},headers)[0]==200;assert json.loads(owner.request(root+'/context')[2])['entries']==[]
    return {'team_published_only':True,'team_multi_service_isolation':True,'team_corrected_revision_and_withdrawal':True,'team_optional_activity_shape_and_end':True,'real_browser_team_workflow':True}
if __name__=='__main__':identity.main(acceptance)
