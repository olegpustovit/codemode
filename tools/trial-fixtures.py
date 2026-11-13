import hashlib, json, os, pathlib, secrets, signal, socket, subprocess, time, urllib.request
root = pathlib.Path.cwd()
state = root / '.tools/trial'
state.mkdir(mode=0o700, exist_ok=True)
suffix = secrets.token_hex(5)
pgdb = 'codemode_spike_' + suffix
mongodb = 'codemode_spike_' + suffix
mongo = 'mongodb://oracle-vm:27017/' + mongodb
pgadmin = 'postgres://snowbear@oracle-vm:5432/postgres?sslmode=disable'
procs = []
pg_created = False

def private(path, content):
    path.write_text(content)
    path.chmod(0o600)

def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]

def request(url, data=None, token=None):
    body = None if data is None else json.dumps(data).encode()
    headers = {'Content-Type':'application/json'}
    if token: headers['Authorization'] = 'Bearer ' + token
    with urllib.request.urlopen(urllib.request.Request(url, body, headers), timeout=5) as r:
        return json.loads(r.read())

def start(name, binary, env):
    log = open(state / (name+'.log'), 'wb')
    p = subprocess.Popen([str(binary), 'serve'] if name == 'artifacts' else [str(binary)], env={**os.environ, **env}, stdout=log, stderr=log)
    procs.append(p)
    print(name + ' process started', flush=True)
    return p

def healthy(name, p, base):
    for _ in range(900):
        if p.poll() is not None: raise RuntimeError(name + ' exited; see fixture log')
        try:
            with urllib.request.urlopen(base+'/healthz', timeout=1) as r:
                if r.status == 200: return
        except Exception: pass
        time.sleep(0.1)
    raise RuntimeError(name+' startup timeout')

signal.signal(signal.SIGTERM, lambda *_: exit(0))

try:
    subprocess.run(['createdb', '--maintenance-db='+pgadmin, pgdb], check=True)
    pg_created = True
    nport, rport, aport = port(), port(), port()
    assert len({nport,rport,aport}) == 3
    nbase, rbase, abase = ['http://127.0.0.1:'+str(p) for p in [nport,rport,aport]]
    nenv = {'NUDGE_MONGO_URI':mongo+'_nudge', 'NUDGE_HOST':'127.0.0.1','NUDGE_PORT':str(nport),'NUDGE_MAIN_ORIGIN':nbase,'NUDGE_AUTH_SECRET':secrets.token_hex(32)}
    renv = {'FOUNDRY_MONGO_URI':mongo+'_researcher','FOUNDRY_LISTEN_ADDR':'127.0.0.1:'+str(rport),'FOUNDRY_ISSUER':rbase,'FOUNDRY_R2_ENDPOINT':'https://example.invalid','FOUNDRY_R2_ACCESS_KEY_ID':'fixture','FOUNDRY_R2_SECRET_ACCESS_KEY':'fixture','FOUNDRY_R2_PRIVATE_BUCKET':'fixture','FOUNDRY_ANALYSIS_ROOT':'/home/snowbear/projects/research-foundry','FOUNDRY_ANALYSIS_STATE_ROOT':str(state/'researcher')}
    aenv = {'ART_DATABASE_URL':'postgres://snowbear@oracle-vm:5432/'+pgdb+'?sslmode=disable','ART_DATA_DIR':str(state/'artifacts-data'),'ART_LISTEN':'127.0.0.1:'+str(aport),'ART_PUBLIC_URL':abase,'ART_COOKIE_SECURE':'false'}
    abin = pathlib.Path('/home/snowbear/projects/artifacts/backend/bin/art')
    art_env = {**os.environ, **aenv}
    subprocess.run([str(abin),'admin','create-user','--email','trial@codemode.test','--name','Codemode Trial','--password',secrets.token_urlsafe(24)], env=art_env, check=True, stdout=subprocess.DEVNULL)
    subprocess.run([str(abin),'admin','create-token','--email','trial@codemode.test','--name','codemode-spike','--write',str(state/'art.conf')], env=art_env, check=True, stdout=subprocess.DEVNULL)
    atoken = dict(line.split('=',1) for line in (state/'art.conf').read_text().splitlines())['token']
    tokens = {'nudge':secrets.token_urlsafe(32),'researcher':secrets.token_urlsafe(32),'artifacts':atoken}
    for name,token in tokens.items(): private(state/(name+'.token'),token)
    seed = '''const n = db.getSiblingDB(process.env.NDB); const r = db.getSiblingDB(process.env.RDB);
    n.service_accounts.insertOne({id:'codemode-trial-agent',workspace_id:'workspace_default',name:'Codemode Trial',token_prefix:'fixture',token_hash:process.env.NHASH,scopes:['read'],active:true,created_by:'fixture',created_at_ms:Date.now()});
    r.owner_auth.insertOne({_id:'owner',salt:'fixture',hash:process.env.PHASH,migration_state:'ready',epoch:0,lock_until:new Date(0)});
    r.oauth_clients.insertOne({_id:'codemode-trial',name:'Codemode Trial',redirect_uris:['http://127.0.0.1/callback'],created:new Date(),disabled:false});
    r.oauth_tokens.insertOne({_id:process.env.RHASH,kind:'access',client:'codemode-trial',epoch:0,audience:process.env.RBASE+'/mcp/ro',exp:new Date(Date.now()+3600000)});
    '''
    (state/'seed.js').write_text(seed)
    seedenv = {**os.environ,'NDB':mongodb+'_nudge','RDB':mongodb+'_researcher','NHASH':hashlib.sha256(tokens['nudge'].encode()).hexdigest(),'RHASH':hashlib.sha256(tokens['researcher'].encode()).hexdigest(),'PHASH':hashlib.pbkdf2_hmac('sha256', secrets.token_bytes(32), b'fixture',600000).hex(),'RBASE':rbase}
    subprocess.run(['mongosh',mongo,'--quiet','--file',str(state/'seed.js')],env=seedenv,check=True,stdout=subprocess.DEVNULL)
    for name,binpath,env,base in [('nudge',root/'.tools/bin/nudge-api',nenv,nbase),('researcher',pathlib.Path('/home/snowbear/projects/research-foundry/dist/linux-arm64/foundry-api'),renv,rbase),('artifacts',abin,aenv,abase)]:
        p = start(name,binpath,env)
        healthy(name,p,base)
    request(nbase+'/api/auth/signup',{'email':'trial@codemode.test','name':'Codemode Trial','password':secrets.token_urlsafe(24)})
    subprocess.run(['mongosh',mongo,'--quiet','--file',str(root/'tools/trial-seed.js')], env={**os.environ,'CODEMODE_TRIAL_NUDGE_DB':mongodb+'_nudge','CODEMODE_TRIAL_RESEARCHER_DB':mongodb+'_researcher'}, check=True)
    for i,title in enumerate(['Worker isolation notes','Gateway deployment checklist','Snapshot restore plan']):
        request(abase+'/api/v1/docs',{'title':title,'markdown':'# '+title+'\n\nTest fixture '+str(i)+'.'},tokens['artifacts'])
    sources = [dict(slug=name,url=base+path,token_file=str(state/(name+'.token'))) for name,base,path in [('nudge',nbase,'/mcp/readonly'),('researcher',rbase,'/mcp/ro'),('artifacts',abase,'/mcp')]]
    private(state/'sources.json',json.dumps(sources))
    print('READY: '+str(state/'sources.json'),flush=True)
    signal.pause()
finally:
    for p in procs:
        p.terminate()
    for p in procs:
        try: p.wait(timeout=10)
        except subprocess.TimeoutExpired: p.kill(); p.wait()
    assert pgdb.startswith('codemode_spike_') and mongodb.startswith('codemode_spike_')
    if pg_created:
        subprocess.run(['dropdb','--if-exists','--maintenance-db='+pgadmin,pgdb],check=True)
    cleanup = 'db.getSiblingDB('+json.dumps(mongodb+'_nudge')+').dropDatabase(); db.getSiblingDB('+json.dumps(mongodb+'_researcher')+').dropDatabase();'
    subprocess.run(['mongosh',mongo,'--quiet','--eval',cleanup],check=True,stdout=subprocess.DEVNULL)
    print('Disposable fixture databases removed',flush=True)
