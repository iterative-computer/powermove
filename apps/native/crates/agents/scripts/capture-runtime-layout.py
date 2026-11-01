import urllib.request, json, tarfile, hashlib, base64, pathlib
root=pathlib.Path(__file__).resolve().parents[5]
(root/'work').mkdir(exist_ok=True)
manifest=json.loads((root/'apps/native/crates/agents/fixtures/runtime-pins.json').read_text())
layouts=[]
for provider,arch in [(p,a) for p in ['codex','claude'] for a in ['arm64','x64']]:
 version='0.159.1-darwin-'+arch if provider=='codex' else '2.1.280'
 package='@openai/codex' if provider=='codex' else '@anthropic-ai/claude-code-darwin-'+arch
 metadata=json.load(urllib.request.urlopen('https://registry.npmjs.org/'+package.replace('/','%2F')+'/'+version))
 pin_name='@openai/codex-darwin-'+arch if provider=='codex' else package
 expected=next(p['integrity'] for p in manifest['pins'] if p['package']==pin_name)
 assert metadata['dist']['integrity']==expected
 archive=root/'work'/(provider+'-'+arch+'.tgz')
 digest=hashlib.sha512()
 with urllib.request.urlopen(metadata['dist']['tarball']) as source, archive.open('wb') as out:
  while chunk:=source.read(65536):
   digest.update(chunk);out.write(chunk)
 assert 'sha512-'+base64.b64encode(digest.digest()).decode()==expected
 with tarfile.open(archive,'r:gz') as tar:
  entries=[{'name':e.name,'kind':e.type.decode(),'mode':oct(e.mode),'size':e.size} for e in tar]
 layouts.append({'package':package+'@'+version,'integrity':expected,'entries':entries})
 print(provider,arch,len(entries),'entries, verified SRI',flush=True)
(root/'apps/native/crates/agents/fixtures/runtime-layout.json').write_text(json.dumps({'source':'public pinned npm tarball headers; no executable launched','layouts':layouts},indent=2)+'\n')
