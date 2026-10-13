#!/usr/bin/env python3
"""Owned synthetic databases only: encrypted snapshot, quarantine and newest journals."""
import json,os,secrets,subprocess,time,urllib.parse,uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];CONTAINER='wk-cursor-native-test'
def main():
    cfg=json.loads(subprocess.check_output(['docker','inspect',CONTAINER]))[0];settings=dict(x.split('=',1) for x in cfg['Config']['Env'] if '=' in x);assert settings.get('POSTGRES_DB')=='wk_test'
    base=os.environ['WK_TEST_DATABASE_URL'];assert urllib.parse.urlparse(base).path=='/wk_test'
    tag=uuid.uuid4().hex;source='wk_hrestore_source_'+tag;target='wk_hrestore_target_'+tag;probe='wk_hrestore_probe_'+tag;work='/tmp/wk-hosted-restore-'+tag;user=settings.get('POSTGRES_USER','postgres');created=[];began=time.monotonic()
    def exec(*args,input=None,check=True):
        r=subprocess.run(['docker','exec','-i',CONTAINER,*args],input=input,capture_output=True,text=True)
        if check and r.returncode:raise RuntimeError('Synthetic restore command failed: '+r.stderr[:500])
        return r
    def sql(db,text):return exec('psql','-X','-At','-v','ON_ERROR_STOP=1','-U',user,'-d',db,input=text).stdout.strip()
    def run(service,*args,check=True):return exec('env','PGSERVICEFILE='+work+'/services','PGSERVICE='+service,*args,check=check)
    def literal(value):return "'"+str(value).replace("'","''")+"'"
    def scoped_url(db):p=urllib.parse.urlparse(base);return urllib.parse.urlunparse(p._replace(path='/'+db))
    def mode(db,command,data=None):
        env=dict(os.environ,WK_DATABASE_URL=scoped_url(db));r=subprocess.run([str(ROOT/'target/debug/wk-server'),command],env=env,input=None if data is None else json.dumps(data),capture_output=True,text=True)
        assert r.returncode==0,r.stderr[:300]
    try:
        exec('mkdir','-m','700',work)
        subprocess.run(['docker','cp',str(ROOT/'deploy/scripts'),CONTAINER+':'+work+'/scripts'],check=True,capture_output=True)
        for db in [source,target]:sql('wk_test','CREATE DATABASE '+db);created.append(db)
        mode(source,'migrate')
        services=''.join('['+name+']\nhost=127.0.0.1\nuser='+user+'\npassword='+settings['POSTGRES_PASSWORD']+'\ndbname='+db+'\n' for name,db in [('source',source),('restore',target)])
        exec('sh','-c','umask 077; cat > "'+work+'/services"',input=services)
        exec('age-keygen','-o',work+'/identity');recipient=exec('age-keygen','-y',work+'/identity').stdout.strip()
        org='restore-'+tag;old=str(uuid.uuid4());kept=str(uuid.uuid4());late=str(uuid.uuid4());card=str(uuid.uuid4());device=str(uuid.uuid4());binding=str(uuid.uuid4());repository=str(uuid.uuid4());preflight=str(uuid.uuid4());operation=str(uuid.uuid4());session=secrets.token_hex(32)
        def body(key):return json.dumps({'schema_version':1,'idempotency_key':key,'policy_revision':1,'title':'Synthetic restore summary','intent':'Bound retained work','rationale':'Scoped middleware','decisions':[],'constraints':[],'open_questions':[],'service_ids':['backend'],'evidence':[],'lifecycle':'proposed','capture_completeness':'complete','base_revision':None})
        sql(source,"INSERT INTO products VALUES("+literal(org)+",'demo'); INSERT INTO memberships(org,product,user_id,services) VALUES("+literal(org)+",'demo','alice',ARRAY['backend']); INSERT INTO identities(issuer,subject,org,user_id) VALUES('https://synthetic-backup.example','alice',"+literal(org)+",'alice'); INSERT INTO product_admins VALUES("+literal(org)+",'demo','alice',ARRAY['backend']); INSERT INTO registered_devices(id,org,product,owner,public_key,approval_hash,services) VALUES("+literal(device)+","+literal(org)+",'demo','alice',"+literal('11'*32)+","+literal('22'*32)+",ARRAY['backend']); INSERT INTO workspace_bindings(id,org,product,user_id,device_id,approval_hash,services,policy_revision) VALUES("+literal(binding)+","+literal(org)+",'demo','alice','synthetic-device',"+literal('22'*32)+",ARRAY['backend'],1); INSERT INTO code_repositories(org,product,id,services) VALUES("+literal(org)+",'demo',"+literal(repository)+",ARRAY['backend']); INSERT INTO code_preflights(id,org,product,owner,device_id,binding_id,security_epoch,services,policy_revision,repository_id,repository_revision,operation_handle,request,source_versions,brief,expires_at,acknowledged_at) VALUES("+literal(preflight)+","+literal(org)+",'demo','alice',"+literal(device)+","+literal(binding)+",1,ARRAY['backend'],1,"+literal(repository)+",1,"+literal(operation)+",'{}','{}','{}',now()+interval '5 minutes',clock_timestamp()); INSERT INTO browser_sessions(session_hash,issuer,subject,csrf_hash) VALUES("+literal(session)+",'https://synthetic-backup.example','alice',"+literal('33'*32)+");")
        for note,key in [(old,'private-old'),(kept,'private-kept')]:sql(source,"INSERT INTO personal_summaries(id,org,product,owner,idempotency_key,services,body,provenance) VALUES("+literal(note)+","+literal(org)+",'demo','alice',"+literal(key)+",ARRAY['backend'],"+literal(body(key))+"::jsonb,'captured')")
        sql(source,"INSERT INTO cards(id,org,product,author,idempotency_key,services,body,published) VALUES("+literal(card)+","+literal(org)+",'demo','alice','shared',ARRAY['backend'],"+literal(body('shared'))+"::jsonb,true)")
        run('source','sh',work+'/scripts/backup.sh',recipient,work+'/backup.age')
        sql(source,"UPDATE personal_summaries SET body=NULL,deleted=true,revision=2 WHERE id="+literal(old)+"; INSERT INTO personal_summaries(id,org,product,owner,idempotency_key,services,body,provenance) VALUES("+literal(late)+","+literal(org)+",'demo','alice','private-late',ARRAY['backend'],"+literal(body('private-late'))+"::jsonb,'captured'); UPDATE personal_summaries SET body=NULL,deleted=true,revision=2 WHERE id="+literal(late)+"; INSERT INTO tombstones(org,product,card_id,revision) VALUES("+literal(org)+",'demo',"+literal(card)+",2); UPDATE cards SET withdrawn=true,published=false WHERE id="+literal(card))
        run('source','sh',work+'/scripts/tombstones.sh','export',recipient,work+'/shared.age')
        # First quarantine is checked before optional later-ledger tools.
        run('restore','sh',work+'/scripts/restore.sh',work+'/identity',work+'/backup.age','--isolated-empty-database')
        assert sql(target,'SELECT count(*) FROM memberships')=='0'
        assert sql(target,'SELECT count(*) FROM product_admins')=='0'
        assert sql(target,'SELECT count(*) FROM browser_sessions')=='0','Restored browser authentication survived'
        assert sql(target,'SELECT count(*) FROM code_preflights')=='0','Restored operation receipt survived quarantine'
        assert sql(target,'SELECT revoked_at IS NOT NULL FROM registered_devices WHERE id='+literal(device))=='t','Restored installation key survived'
        assert int(sql(target,'SELECT not_before FROM collector_security_epoch'))>0
        sql('wk_test','CREATE ROLE '+probe+' NOLOGIN')
        assert sql(target,"SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND has_function_privilege("+literal(probe)+",p.oid,'EXECUTE')")=='0','Restored security-definer helpers gained PUBLIC execution'
        sql('wk_test','DROP ROLE '+probe);probe=None
        run('source','sh',work+'/scripts/personal-deletions.sh','export',recipient,work+'/personal.age')
        assert run('source','sh',work+'/scripts/personal-deletions.sh','export',recipient,work+'/personal.age',check=False).returncode!=0
        run('restore','sh',work+'/scripts/tombstones.sh','merge',work+'/identity',work+'/shared.age')
        run('restore','sh',work+'/scripts/personal-deletions.sh','merge',work+'/identity',work+'/personal.age')
        assert sql(target,'SELECT deleted AND body IS NULL AND revision>=2 FROM personal_summaries WHERE id='+literal(old))=='t'
        assert sql(target,'SELECT NOT deleted AND body IS NOT NULL FROM personal_summaries WHERE id='+literal(kept))=='t'
        assert sql(target,'SELECT withdrawn AND NOT published FROM cards WHERE id='+literal(card))=='t'
        assert sql(target,'SELECT count(*) FROM personal_deletions WHERE note_id='+literal(late))=='1'
        run('restore','sh',work+'/scripts/personal-deletions.sh','merge',work+'/identity',work+'/personal.age')
        # Regrant only after both independent latest journals have been merged.
        mode(target,'provision',{'issuer':'https://synthetic-backup.example','subject':'alice','org':org,'user':'alice','product':'demo','services':['backend'],'can_review':False})
        assert sql(target,'SELECT revoked_at IS NOT NULL FROM registered_devices WHERE id='+literal(device))=='t'
        assert sql(target,"BEGIN; SELECT set_config('wk.org',"+literal(org)+",true); SELECT set_config('wk.user','alice',true); SELECT wk_personal_deleted_key("+literal(org)+",'demo','alice','private-late'); ROLLBACK;").splitlines()[-2]=='t'
        assert run('restore','sh',work+'/scripts/restore.sh',work+'/identity',work+'/backup.age','--isolated-empty-database',check=False).returncode!=0
        exec('sh','-c','printf malformed | age -r "'+recipient+'" -o "'+work+'/bad.age"')
        assert run('restore','sh',work+'/scripts/personal-deletions.sh','merge',work+'/identity',work+'/bad.age',check=False).returncode!=0
        assert sql(target,'SELECT count(*) FROM personal_deletions')=='2'
        # Malformed rows with a valid header must roll back the entire merge too.
        exec('sh','-c','age -r "'+recipient+'" -o "'+work+'/bad-row.age"',input='org,product,owner,note_id,revision,key_hash,deleted_at\nmalformed\n')
        assert run('restore','sh',work+'/scripts/personal-deletions.sh','merge',work+'/identity',work+'/bad-row.age',check=False).returncode!=0
        assert sql(target,'SELECT count(*) FROM personal_deletions')=='2'
        # Fault injection is confined to a private copy of this script and a third new DB.
        fault='wk_hrestore_fault_'+tag;sql('wk_test','CREATE DATABASE '+fault);created.append(fault)
        services+='[fault]\nhost=127.0.0.1\nuser='+user+'\npassword='+settings['POSTGRES_PASSWORD']+'\ndbname='+fault+'\n'
        exec('sh','-c','umask 077; cat > "'+work+'/services"',input=services)
        script=(ROOT/'deploy/scripts/restore.sh').read_text().replace('COMMIT;','SELECT 1/0;\nCOMMIT;')
        exec('sh','-c','cat > "'+work+'/fault-restore.sh"',input=script)
        assert run('fault','sh',work+'/fault-restore.sh',work+'/identity',work+'/backup.age','--isolated-empty-database',check=False).returncode!=0
        assert sql(fault,"SELECT count(*) FROM pg_tables WHERE schemaname='public'")=='0','Interrupted restore left committed tables'
        # Destroy only a copy of our ciphertext; decryption must fail before restore.
        exec('cp',work+'/backup.age',work+'/tampered.age')
        exec('sh','-c','printf X | dd of="'+work+'/tampered.age" bs=1 seek=0 conv=notrunc status=none')
        assert run('fault','sh',work+'/scripts/restore.sh',work+'/identity',work+'/tampered.age','--isolated-empty-database',check=False).returncode!=0
        assert sql(fault,"SELECT count(*) FROM pg_tables WHERE schemaname='public'")=='0'
        exec('sh','-c','printf "wk-backup-v1\\n" > "'+work+'/manifest.txt"; age -d -i "'+work+'/identity" "'+work+'/backup.age" | tar -xOf - database.dump > "'+work+'/database.dump"; tar -C "'+work+'" -cf - database.dump manifest.txt | age -r "'+recipient+'" -o "'+work+'/old-version.age"')
        assert run('fault','sh',work+'/scripts/restore.sh',work+'/identity',work+'/old-version.age','--isolated-empty-database',check=False).returncode!=0
        assert sql(fault,"SELECT count(*) FROM pg_tables WHERE schemaname='public'")=='0'
        versions={name:exec(name,'--version').stdout.strip() for name in ['age','pg_dump','pg_restore']}
        print(json.dumps({'passed':{'encrypted_snapshot':True,'atomic_authority_quarantine':True,'collector_auth_cutoff':True,'preflight_receipts_invalidated':True,'no_public_definer_execution':True,'latest_shared_and_private_replay':True,'orphan_deleted_key_protected':True,'unrelated_private_note_preserved':True,'repeat_merge_idempotent':True,'nonempty_restore_refused':True,'invalid_ledger_rejected':True,'interrupted_restore_rolls_back':True,'tampered_ciphertext_rejected':True,'incompatible_manifest_refused':True},'versions':versions,'elapsed_seconds':round(time.monotonic()-began,2),'source_and_target':'new disposable synthetic databases only'}))
    finally:
        if probe is not None:exec('psql','-X','-U',user,'-d','wk_test','-c','DROP ROLE IF EXISTS '+probe,check=False)
        for db in reversed(created):exec('psql','-X','-U',user,'-d','wk_test','-c','DROP DATABASE IF EXISTS '+db+' WITH (FORCE)',check=False)
        exec('rm','-rf',work,check=False)
if __name__=='__main__':main()
