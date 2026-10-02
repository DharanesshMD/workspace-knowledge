"""Generate checksums and a dependency SPDX inventory for a curated release directory."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import uuid

ROOT=Path(__file__).resolve().parents[1]
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('directory',type=Path);args=parser.parse_args()
    directory=args.directory.resolve(strict=True)
    if directory==ROOT or ROOT in directory.parents and directory.name in ['target','web','.tooling']:
        raise ValueError('use a separate curated release directory')
    version=subprocess.check_output([str(ROOT/'scripts/cargo'),'-vV'],text=True)
    host=next(line.split(': ',1)[1] for line in version.splitlines() if line.startswith('host: '))
    metadata=json.loads(subprocess.check_output([str(ROOT/'scripts/cargo'),'metadata','--locked','--offline','--format-version','1','--filter-platform',host],cwd=ROOT))
    packages=[{'SPDXID':'SPDXRef-Package-'+str(i),'name':p['name'],'versionInfo':p['version'],'downloadLocation':'NOASSERTION','filesAnalyzed':False,'licenseConcluded':'NOASSERTION','licenseDeclared':p.get('license') or 'NOASSERTION','copyrightText':'NOASSERTION'} for i,p in enumerate(metadata['packages'])]
    sbom={'spdxVersion':'SPDX-2.3','dataLicense':'CC0-1.0','SPDXID':'SPDXRef-DOCUMENT','name':'workspace-knowledge-dependency-inventory','documentNamespace':'https://spdx.org/spdxdocs/workspace-knowledge-'+str(uuid.uuid4()),'creationInfo':{'creators':['Tool: workspace-knowledge/release-manifest'],'created':datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')},'packages':packages}
    (directory/'dependencies.spdx.json').write_text(json.dumps(sbom,indent=2)+'\n')
    hashes=[]
    for file in sorted(directory.rglob('*')):
        if file.is_symlink():raise ValueError('release directory must not contain symlinks')
        if file.is_file() and file.name not in ['SHA256SUMS','SHA256SUMS.sig']:
            name=file.relative_to(directory).as_posix()
            if '\n' in name or '\r' in name:raise ValueError('invalid artifact name')
            hashes.append(hashlib.sha256(file.read_bytes()).hexdigest()+'  '+name)
    (directory/'SHA256SUMS').write_text('\n'.join(hashes)+'\n')
    print('Checksums and dependency inventory written; no signature or vulnerability attestation implied')
if __name__=='__main__':main()
