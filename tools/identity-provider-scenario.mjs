import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createHash, randomBytes } from 'node:crypto';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { spawn, execFileSync } from 'node:child_process';

// Called only by Identity's owned native PG18 runner with ID_E2E_TLS=1. All
// accounts, email, databases, keys and services are temporary.
export async function lifecycle({ browser, base, certificate, ownedHttps, mail, root, fixtureSql, setupCode }) {
  assert(new URL(base).protocol === 'https:' && new URL(base).hostname === 'localhost');
  assert(certificate && typeof ownedHttps === 'function' && typeof fixtureSql === 'function');
  const owner = await browser.newContext({ ignoreHTTPSErrors: true });
  let member = await browser.newContext({ ignoreHTTPSErrors: true });
  for (const context of [owner,member]) context.setDefaultTimeout(10_000);
  const admin = await owner.newPage();
  let person = await member.newPage();
  const errors = [];
  for (const context of [owner,member]) context.on('page',page=>page.on('pageerror',()=>errors.push('browser exception')));
  person.on('pageerror',()=>errors.push('browser exception'));
  const password = randomBytes(24).toString('base64url');
  const email = 'eval-codemode@olegpustovit.com';
  let stage = 'setup identity';
  let child, proxy, mappedBrowser, missingPolicy = false;
  const actions = [];
  const callback = createServer((request,response)=> {
    response.writeHead(200,{'content-type':'text/plain'}); response.end('Owned native callback');
  });
  await new Promise((ok,fail)=>{ callback.once('error',fail); callback.listen(0,'127.0.0.1',ok); });
  const callbackURL = `http://127.0.0.1:${callback.address().port}/callback`;
  async function visible(locator) { await locator.waitFor({state:'visible'}); }
  async function secret(page,selector,value) {
    await visible(page.locator(selector));
    await page.evaluate(({selector,value})=>{
      const input=document.querySelector(selector);
      if(!(input instanceof HTMLInputElement))throw Error('missing secret input');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,value);
      input.dispatchEvent(new Event('input',{bubbles:true}));
      input.dispatchEvent(new Event('change',{bubbles:true}));
    },{selector,value});
  }
  async function stop() {
    if(!child || child.exitCode!==null || child.signalCode!==null)return;
    const exited=new Promise(ok=>child.once('exit',ok));
    child.kill('SIGTERM'); const timer=setTimeout(()=>child.kill('SIGKILL'),5000);
    try { await exited; } finally { clearTimeout(timer); }
  }
  try {
    await admin.goto(base+'/setup');
    await secret(admin,'[name=code]',setupCode);
    await admin.getByRole('button',{name:'Continue',exact:true}).click();
    await admin.locator('[name=name]').fill('Eval native owner');
    await admin.locator('[name=email]').fill('eval-owner@example.test');
    await secret(admin,'[name=password]',password);
    await admin.locator('[name=passkey]').uncheck();
    await admin.getByRole('button',{name:'Create account',exact:true}).click();
    await admin.waitForURL(base+'/welcome');
    stage='authorize identity admin MCP';
    const registered=await owner.request.post(base+'/register',{data:{client_name:'Codemode owned native',redirect_uris:[callbackURL],token_endpoint_auth_method:'none'}});
    assert.equal(registered.status(),201);
    const mcpID=(await registered.json()).client_id;
    const verifier=randomBytes(32).toString('base64url');
    const state=randomBytes(24).toString('base64url');
    const query=new URLSearchParams({client_id:mcpID,redirect_uri:callbackURL,response_type:'code',scope:'mcp:admin',state,resource:base+'/mcp',code_challenge:createHash('sha256').update(verifier).digest('base64url'),code_challenge_method:'S256'});
    await admin.goto(base+'/authorize?'+query);
    await admin.locator('button[name=decision][value=continue]').click();
    await admin.getByRole('button',{name:'Allow',exact:true}).click();
    await admin.waitForURL(callbackURL+'**');
    const back=new URL(admin.url()); assert(back.searchParams.get('state')===state && back.searchParams.has('code'));
    const exchanged=await owner.request.post(base+'/token',{form:{client_id:mcpID,grant_type:'authorization_code',code:back.searchParams.get('code'),redirect_uri:callbackURL,code_verifier:verifier,resource:base+'/mcp'}});
    assert.equal(exchanged.status(),200);
    const centralToken=(await exchanged.json()).access_token;
    const headers={authorization:'Bearer '+centralToken,accept:'application/json, text/event-stream','mcp-protocol-version':'2025-11-25'};
    let requestID=0;
    async function rpc(method,params,notification=false) {
      const id=notification?undefined:++requestID;
      const response=await owner.request.post(base+'/mcp',{headers,data:{jsonrpc:'2.0',...(notification?{}:{id}),method,params},timeout:10_000});
      if(notification){assert([200,202,204].includes(response.status()));return;}
      assert.equal(response.status(),200);
      const session=response.headers()['mcp-session-id']; if(session)headers['mcp-session-id']=session;
      const text=await response.text();
      const message=response.headers()['content-type'].includes('application/json')?JSON.parse(text):text.split('\n').filter(line=>line.startsWith('data:')).map(line=>JSON.parse(line.slice(5))).find(value=>value.id===id);
      assert(message?.result&&!message.error,'native Identity MCP failed'); return message.result;
    }
    await rpc('initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'codemode-owned-native',version:'1'}});
    await rpc('notifications/initialized',{},true);
    async function tool(name,args) { const result=await rpc('tools/call',{name,arguments:args}); assert(!result.isError&&result.structuredContent,'native Identity tool failed'); return result.structuredContent; }
    stage='register native app';
    const reservation=createServer();
    await new Promise((ok,fail)=>{reservation.once('error',fail);reservation.listen(0,'127.0.0.1',ok);});
    const port=reservation.address().port;
    await new Promise(ok=>reservation.close(ok));
    proxy=await ownedHttps({root,name:'codemode',target:`http://127.0.0.1:${port}`});
    const hostname='eval-codemode.example.test', origin='https://'+hostname;
    const tlsPort=new URL(proxy.base).port;
    // Chromium-only DNS mapping keeps canonical HTTPS URLs and callback/Origin
    // validation intact, without host-file, Caddy, DNS or privileged-port writes.
    mappedBrowser=await browser.browserType().launch({args:['--no-sandbox','--no-proxy-server',`--host-resolver-rules=MAP ${hostname} 127.0.0.1:${tlsPort}`]});
    const moved=await mappedBrowser.newContext({ignoreHTTPSErrors:true,storageState:await member.storageState()});
    await member.close(); member=moved; member.setDefaultTimeout(10_000);
    person=await member.newPage();
    person.on('pageerror',()=>errors.push('browser exception'));
    const created=await tool('app_create',{kind:'confidential',settings:{name:'Eval Codemode',home_url:origin,redirect_uris:[origin+'/auth/callback'],trusted:true,refresh:false}});
    assert(created.app.id&&created.client_secret);
    stage='captured verified invitation';
    assert((await tool('invite_create',{email,max_uses:1,valid_hours:72})).email_sent);
    let invite; const deadline=Date.now()+5000;
    while(!invite&&Date.now()<deadline) {
      for(const name of await readdir(mail)) { const message=JSON.parse(await readFile(join(mail,name),'utf8')); if(message.to.includes(email))invite=message.text.split('\n').find(line=>line.startsWith(base+'/join/')); }
      if(!invite)await new Promise(ok=>setTimeout(ok,50));
    }
    assert(invite,'captured eval invitation missing');
    await person.goto(invite);
    await person.locator('[name=name]').fill('Eval Codemode member');
    await secret(person,'[name=password]',password);
    await person.locator('[name=passkey]').uncheck();
    await person.getByRole('button',{name:'Create account',exact:true}).click();
    await person.waitForURL(base+'/joined');
    const idCookie=(await member.cookies(base)).find(c=>c.name==='__Host-id')?.value;
    assert(idCookie,'native ID device cookie missing');
    stage='reviewed app policy';
    await admin.goto(base+'/access');
    const accessCSRF=await admin.locator('#access-page').getAttribute('data-csrf'); assert(accessCSRF);
    async function policy(changes,reason) {
      const current=await owner.request.get(base+'/admin/access/summary'); assert.equal(current.status(),200);
      const response=await owner.request.post(base+'/admin/access/changes',{headers:{Origin:base,'X-CSRF-Token':accessCSRF},data:{changes:{expected_revision:(await current.json()).revision,groups:[],overrides:[],...changes},reason,idempotency_key:randomBytes(20).toString('hex'),acknowledge_broad_masks:false}});
      assert.equal(response.status(),200,'reviewed native policy failed');
    }
    await policy({groups:[{id:'eval-codemode-native',name:'eval Codemode native',description:'Owned captured verification',archived:false,selectors:[email],members:[],apps:[created.app.id]}]},'eval reviewed Codemode native access');
    const ids=JSON.parse(fixtureSql('COPY (SELECT json_object_agg(email::text,id) FROM users) TO STDOUT').trim());
    const subject=ids[email]; assert(subject);
    async function action(name) {
      actions.push(name);
      if(name==='block'||name==='restore')await policy({overrides:[{user_id:subject,client_id:created.app.id,decision:name==='block'?'block':null}]},'eval reviewed Codemode '+name);
      else if(name==='outage'){ assert(!missingPolicy); fixtureSql('ALTER TABLE access_policy_state RENAME TO eval_codemode_policy_unavailable'); missingPolicy=true; }
      else if(name==='recover'){ assert(missingPolicy); fixtureSql('ALTER TABLE eval_codemode_policy_unavailable RENAME TO access_policy_state'); missingPolicy=false; }
      else throw Error('invalid owned action');
    }
    stage='start enforced native Codemode';
    assert(root.includes('/identity-e2e-')&&new URL(base).hostname==='localhost','owned runner required');
    fixtureSql('CREATE DATABASE codemode_eval_native');
    const pgPort=fixtureSql("COPY (SELECT current_setting('port')) TO STDOUT").trim();
    assert(/^\d+$/.test(pgPort));
    const database=`postgresql://identity_e2e@127.0.0.1:${pgPort}/codemode_eval_native`;
    const repo=resolve(import.meta.dirname,'..');
    function sql(query) { return execFileSync('psql',['-X','-v','ON_ERROR_STOP=1','-d',database,'-Atqc',query],{encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim(); }
    await writeFile(join(root,'codemode-sources.toml'),'sources = []\n',{mode:0o600});
    const encryptionKey=randomBytes(32).toString('hex'), cookieKey=randomBytes(32).toString('hex');
    async function start(enforced) {
      child=spawn(resolve(repo,'target/debug/codemode'),[],{cwd:repo,stdio:['ignore','ignore','ignore'],env:{...process.env,
        CODEMODE_DATABASE_URL:database,CODEMODE_HOSTNAME:hostname,CODEMODE_BIND:`127.0.0.1:${port}`,CODEMODE_SOURCES_FILE:join(root,'codemode-sources.toml'),
        CODEMODE_MONTY_BIN:resolve(repo,'.tools/monty/bin/monty'),CODEMODE_ENCRYPTION_KEY:encryptionKey,CODEMODE_ID_COOKIE_KEY:cookieKey,
        CODEMODE_ID_ENABLED:'true',CODEMODE_ID_ACCESS_ENABLED:String(enforced),CODEMODE_ID_ISSUER:base,CODEMODE_ID_CLIENT_ID:created.app.id,CODEMODE_ID_CLIENT_SECRET:created.client_secret,SSL_CERT_FILE:certificate,
      }});
      child.on('error',()=>{});
      const deadline=Date.now()+10000; let ready=false, lastStatus, lastMode, lastKeys;
      while(Date.now()<deadline) {
        assert(child.exitCode===null&&child.signalCode===null,'owned Codemode exited');
        try { const response=await fetch(`http://127.0.0.1:${port}/healthz`,{headers:{host:hostname}}); const value=await response.json(); lastStatus=response.status; lastMode=value.identityAccessEnforced; lastKeys=Object.keys(value).join(","); ready=response.ok&&value.identityAccessEnforced===enforced; } catch {}
        if(ready)break; await new Promise(ok=>setTimeout(ok,50));
      }
      assert(ready,`owned Codemode readiness status=${lastStatus} mode=${lastMode} keys=${lastKeys}`);
    }
    await start(true);
    async function api(path,method='GET',data,headers={}) {
      return person.evaluate(async ({path,method,data,headers})=>{
        const response=await fetch(path,{method,credentials:'include',headers:{...(data!==undefined?{'content-type':'application/json'}:{}),...headers},...(data!==undefined?{body:JSON.stringify(data)}:{})});
        const text=await response.text(); let value; try{value=JSON.parse(text);}catch{}
        return {status:response.status,value};
      },{path,method,data,headers});
    }
    async function signIn() {
      await person.goto(origin+'/auth/login');
      await person.waitForURL(url=>url.origin===new URL(base).origin||url.origin===origin);
      if(new URL(person.url()).origin===new URL(base).origin)await person.locator('button[name=decision][value=continue]').click();
      await person.waitForURL(url=>url.origin===origin&&url.pathname!=='/auth/callback');
      assert.equal((await api('/api/session')).status,200);
    }
    stage='native confidential OIDC callback';
    await signIn();
    const session=await api('/api/session'); assert(session.value.authenticated);
    const local=session.value.user; assert(local.id&&local.tenantId&&local.email===email&&local.admin===false);
    assert.equal(sql('SELECT count(*) FROM user_identities'), '1');
    assert.equal(sql('SELECT subject FROM user_identities'),subject);
    const saved=await api('/api/workspaces','POST',{name:'eval-native',sourceIds:[]}); assert.equal(saved.status,201); assert(saved.value.id);
    const token=await api('/api/tokens','POST',{name:'eval-native-pat',workspaceId:saved.value.id,administration:false}); assert.equal(token.status,201); assert(token.value.token);
    const pat=token.value.token;
    async function call(bearer,method='tools/list',params={},path='/mcp') {
      return api(path,'POST',{jsonrpc:'2.0',id:1,method,params},{Authorization:'Bearer '+bearer,Accept:'application/json, text/event-stream','MCP-Protocol-Version':'2025-11-25'});
    }
    const tools=await call(pat); assert.equal(tools.status,200); assert.deepEqual(tools.value.result.tools.map(t=>t.name).sort(),['execute','search']);
    const executed=await call(pat,'tools/call',{name:'execute',arguments:{code:'return 42'}}); assert.equal(executed.status,200); assert(!executed.value.result.isError);
    assert.equal((await call(pat,'tools/list',{},'/mcp-admin')).status,401);
    stage='native local OAuth consent';
    const appCallback=origin+'/eval-callback'; let callbackCode;
    await person.route('**/eval-callback?**',async route=>{callbackCode=new URL(route.request().url());await route.fulfill({status:200,contentType:'text/plain',body:'Owned Codemode OAuth complete'});});
    const agentRegistered=await api('/oauth/register','POST',{client_name:'Eval native agent',redirect_uris:[appCallback],token_endpoint_auth_method:'none',grant_types:['authorization_code','refresh_token'],response_types:['code']}); assert.equal(agentRegistered.status,201);
    const client=agentRegistered.value.client_id, verifier2=randomBytes(32).toString('base64url'), state2=randomBytes(20).toString('hex');
    const query2=new URLSearchParams({response_type:'code',client_id:client,redirect_uri:appCallback,resource:origin+'/mcp',scope:'mcp',code_challenge:createHash('sha256').update(verifier2).digest('base64url'),code_challenge_method:'S256',state:state2});
    await person.goto(origin+'/oauth/authorize?'+query2);
    await person.getByLabel('Workspace',{exact:true}).selectOption(saved.value.id);
    await person.getByRole('button',{name:'Authorize workspace',exact:true}).click();
    await person.getByText('Owned Codemode OAuth complete',{exact:true}).waitFor();
    assert(callbackCode?.searchParams.get('state')===state2&&callbackCode.searchParams.has('code'));
    async function form(data) {
      return person.evaluate(async data=>{const response=await fetch('/oauth/token',{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams(data)});return {status:response.status,value:await response.json()};},data);
    }
    const exchange={client_id:client,grant_type:'authorization_code',resource:origin+'/mcp',code:callbackCode.searchParams.get('code'),code_verifier:verifier2,redirect_uri:appCallback};
    for(const [name,status] of [['block',403],['outage',503],['recover',403]]) { await action(name); assert.equal((await form(exchange)).status,status); }
    await action('restore');
    let exchanged2=await form(exchange); assert.equal(exchanged2.status,200); let tokens=exchanged2.value;
    assert(tokens.access_token&&tokens.refresh_token&&tokens.scope==='mcp');
    const identityBefore=sql("SELECT json_agg(u ORDER BY id)::text FROM (SELECT id,email,tenant_id,admin FROM users) u");
    const decisions=[];
    for(const [name,expected] of [['block',403],['restore',200],['outage',503],['recover',200]]) {
      await action(name);
      assert.equal((await api('/api/workspaces')).status,expected);
      assert.equal((await call(pat)).status,expected);
      assert.equal((await call(tokens.access_token)).status,expected);
      const refreshed=await form({client_id:client,grant_type:'refresh_token',resource:origin+'/mcp',refresh_token:tokens.refresh_token});
      assert.equal(refreshed.status,expected);
      if(expected===200)tokens=refreshed.value;
      assert.equal(sql("SELECT json_agg(u ORDER BY id)::text FROM (SELECT id,email,tenant_id,admin FROM users) u"),identityBefore);
      assert.equal(sql("SELECT count(*) FROM endpoints WHERE slug='eval-native'"),'1');
      decisions.push({action:name,status:expected});
    }
    stage='blocked app logout preserves global identity';
    const beforeID=(await member.cookies(base)).find(c=>c.name==='__Host-id')?.value;
    await action('block');
    assert.equal((await api('/api/session','DELETE')).status,204);
    assert(!(await member.cookies(origin)).some(c=>c.name==='__Host-codemode_id_session'));
    assert((await member.cookies(base)).find(c=>c.name==='__Host-id')?.value===beforeID,'app logout changed the global ID device');
    await action('restore'); await signIn();
    assert.equal(sql('SELECT count(*) FROM users'),'1');
    stage='explicit default-off and reenable preserve credentials';
    await stop(); await start(false); await action('block');
    assert.equal((await api('/api/workspaces')).status,200); assert.equal((await call(pat)).status,200);
    await stop(); await start(true);
    assert.equal((await api('/api/workspaces')).status,403); assert.equal((await call(pat)).status,403);
    await action('restore');
    assert.equal((await api('/api/workspaces')).status,200); assert.equal((await call(pat)).status,200);
    assert.equal(sql("SELECT json_agg(u ORDER BY id)::text FROM (SELECT id,email,tenant_id,admin FROM users) u"),identityBefore);
    assert.deepEqual(errors,[]);
    await person.goto(base+'/account');
    const idCSRF=await person.locator('form[action="/signout"] [name=csrf]').inputValue();
    assert.equal((await member.request.post(base+'/signout',{form:{csrf:idCSRF},maxRedirects:0})).status(),303);
    const people=await tool('people_list',{}); assert(people.people.find(user=>user.email===email)?.devices===0);
    if(process.env.CODEMODE_ID_REHEARSAL_REPORT)await writeFile(process.env.CODEMODE_ID_REHEARSAL_REPORT,JSON.stringify({scope:'Actual Identity/Codemode HTTPS daemons, owned PostgreSQL 18 and Chromium-only DNS mapping',verification:'captured email invitation',policy:'reviewed protected API',actions,decisions,confidential_oidc:true,cookie_pat_oauth:true,code_refresh_preserved_on_denial_outage:true,workspace_resource_scope_preserved:true,exact_two_execution_tools:true,actual_worker:true,blocked_logout_preserves_id_device:true,subject_user_tenant_role_and_data_preserved:true,explicit_default_off_and_reenable:true,leftover_id_devices:0},null,2)+'\n',{mode:0o600});
    console.log('Codemode native ID: verified invitation, reviewed policy, HTTPS OIDC, cookie/PAT/OAuth/refresh, real Block/outage/restoration, worker, resource scope and blocked logout passed');
  } catch(error) {
    throw Error('Codemode native qualification failed at '+stage+': '+error.name);
  } finally {
    if(missingPolicy)fixtureSql('ALTER TABLE eval_codemode_policy_unavailable RENAME TO access_policy_state');
    await stop(); await proxy?.close();
    await new Promise(ok=>{callback.close(ok);callback.closeAllConnections();});
    await owner.close(); await member.close(); await mappedBrowser?.close();
  }
}
