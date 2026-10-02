"""Disposable synthetic encrypted backup/restore drill. Requires wk-completion-postgres with age installed."""
import json
from pathlib import Path
import subprocess

ROOT=Path(__file__).resolve().parents[1]
CONTAINER='wk-completion-postgres'
def execute(*args,**kwargs):
    return subprocess.run(['docker','exec',CONTAINER,*args],check=True,capture_output=True,text=True,**kwargs).stdout

def main():
    execute('mkdir','-p','/tmp/wk-drill')
    subprocess.run(['docker','cp',str(ROOT/'deploy/scripts'),CONTAINER+':/tmp/wk-drill/scripts'],check=True,capture_output=True)
    service='[source]\nhost=127.0.0.1\nuser=wk_test\npassword=wk-test-only\ndbname=wk_test\n[restore]\nhost=127.0.0.1\nuser=wk_test\npassword=wk-test-only\ndbname=wk_restore\n'
    subprocess.run(['docker','exec','-i',CONTAINER,'sh','-c','umask 077; cat > /tmp/wk-drill/services'],input=service,text=True,check=True)
    execute('age-keygen','-o','/tmp/wk-drill/identity')
    recipient=execute('age-keygen','-y','/tmp/wk-drill/identity').strip()
    def run(service,*args): return execute('env','PGSERVICEFILE=/tmp/wk-drill/services','PGSERVICE='+service,*args)
    marker=run('source','psql','-X','-At','-v','ON_ERROR_STOP=1','-c',"SELECT json_build_object('org',org,'product',product,'id',id)::text FROM cards WHERE NOT withdrawn AND published LIMIT 1").strip()
    chosen=json.loads(marker)
    run('source','sh','/tmp/wk-drill/scripts/backup.sh',recipient,'/tmp/wk-drill/backup.age')
    # The identifiers below originate from our synthetic database, with literals escaped.
    def literal(s): return "'"+s.replace("'","''")+"'"
    condition=f"org={literal(chosen['org'])} AND product={literal(chosen['product'])} AND id={literal(chosen['id'])}"
    run('source','psql','-X','-v','ON_ERROR_STOP=1','-c',f"INSERT INTO tombstones(org,product,card_id,revision) SELECT org,product,id,revision FROM cards WHERE {condition}; UPDATE cards SET withdrawn=true,published=false WHERE {condition}")
    run('source','sh','/tmp/wk-drill/scripts/tombstones.sh','export',recipient,'/tmp/wk-drill/latest.age')
    execute('createdb','-U','wk_test','wk_restore')
    run('restore','sh','/tmp/wk-drill/scripts/restore.sh','/tmp/wk-drill/identity','/tmp/wk-drill/backup.age','--isolated-empty-database')
    assert run('restore','psql','-X','-At','-c','SELECT count(*) FROM memberships').strip()=='0'
    assert run('restore','psql','-X','-At','-c',f'SELECT withdrawn FROM cards WHERE {condition}').strip()=='f'
    run('restore','sh','/tmp/wk-drill/scripts/tombstones.sh','merge','/tmp/wk-drill/identity','/tmp/wk-drill/latest.age')
    assert run('restore','psql','-X','-At','-c',f'SELECT withdrawn AND NOT published FROM cards WHERE {condition}').strip()=='t'
    retry=subprocess.run(['docker','exec',CONTAINER,'env','PGSERVICEFILE=/tmp/wk-drill/services','PGSERVICE=restore','sh','/tmp/wk-drill/scripts/restore.sh','/tmp/wk-drill/identity','/tmp/wk-drill/backup.age','--isolated-empty-database'],capture_output=True)
    assert retry.returncode!=0,'restore accepted a nonempty database'
    print('PASS: encrypted snapshot, atomic permission removal, latest tombstone replay, nonempty restore refusal')
if __name__=='__main__': main()
