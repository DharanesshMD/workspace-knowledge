import {test,expect,type Page,type Route} from '@playwright/test';
import {readFileSync} from 'node:fs';
const fixture=(name:string)=>JSON.parse(readFileSync(new URL(`./fixtures/${name}`,import.meta.url),'utf8'));
const url='https://wk.example/mcp/alpha';
const secret=`wkpt_${'cd34'.repeat(16)}`;
const secret2=`wkpt_${'ef56'.repeat(16)}`;
const day=86_400_000;
type Tok={id:string;label:string;created_ms:number;expires_ms:number;last_used_ms:number|null};
type User={user:string;services:string[];can_review:boolean;token_only:boolean;tokens:Tok[]};
const tok:Tok={id:'11111111-1111-4111-8111-111111111111',label:'issued by admin',created_ms:Date.UTC(2026,8,1),expires_ms:Date.UTC(2026,10,1),last_used_ms:null};
const sam:User={user:'sam',services:['backend'],can_review:false,token_only:true,tokens:[tok]};
const ravi:User={user:'ravi',services:['backend','frontend'],can_review:true,token_only:false,tokens:[]};

type Opts={admin?:boolean;settings?:{max_days:number}|null;users?:User[];mcp?:string|null;create?:(r:Route)=>Promise<void>|void};
async function setup(page:Page,o:Opts={}){
 const state={users:o.users??[sam,ravi],calls:[] as {method:string;path:string;csrf?:string;body:unknown}[]};
 const admin=o.admin??true;
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({json:{user:'owner',org:'team',csrf_token:'csrf-9',personal_tokens:o.settings===undefined?{max_days:90}:o.settings,
  products:[{product:'alpha',services:['backend','frontend'],can_review:true,policy_revision:1,mcp_url:url,is_admin:admin}]}}));
 await page.route('**/personal',r=>r.fulfill({json:{summaries:[],next_cursor:null}}));
 await page.route('**/v1/products/alpha/agent-setup**',r=>r.fulfill({json:fixture(r.request().url().includes('auto_share=true')?'agent-setup-auto.json':'agent-setup.json')}));
 await page.route('**/v1/tokens**',r=>r.fulfill({json:{tokens:[]}}));
 await page.route('**/v1/admin/products/alpha/users**',async r=>{
  const req=r.request();const path=new URL(req.url()).pathname;
  state.calls.push({method:req.method(),path,csrf:req.headers()['x-csrf-token'],body:req.postDataJSON()});
  const mcp_url=o.mcp===undefined?url:o.mcp;
  if(req.method()==='GET')return r.fulfill({json:{services:['backend','frontend'],users:state.users,mcp_url}});
  if(req.method()==='DELETE'){state.users=state.users.filter(u=>!path.endsWith(`/${u.user}`));return r.fulfill({json:{removed:true}});}
  if(path.endsWith('/token')){const name=path.split('/').at(-2)!;return r.fulfill({json:{user:name,product:'alpha',token:secret2,id:'x',label:'reissued by admin',created_ms:Date.now(),expires_ms:Date.now()+30*day,mcp_url}});}
  if(o.create)return o.create(r);
  const b=req.postDataJSON();
  state.users=[...state.users,{user:b.user,services:b.services,can_review:b.can_review,token_only:true,tokens:[{...tok,id:'22222222-2222-4222-8222-222222222222',expires_ms:Date.now()+b.days*day}]}];
  return r.fulfill({status:201,json:{user:b.user,product:'alpha',token:secret,id:'y',label:'issued by admin',created_ms:Date.now(),expires_ms:Date.now()+b.days*day,mcp_url}});
 });
 await page.goto('/');
 return state;
}
const nav=(page:Page)=>page.getByRole('navigation',{name:'Main navigation'});
const open=async(page:Page)=>{await nav(page).getByRole('button',{name:'Admin',exact:true}).click();await expect(page.getByRole('heading',{name:'Admin',level:1})).toBeVisible();};
const copyButton=(page:Page,label:string)=>page.getByRole('button',{name:new RegExp(`^(Copy|Copied) ${label}$`)});

for(const [name,opts] of [['a developer who is not an admin',{admin:false}],['a server with personal tokens off',{settings:null}]] as [string,Opts][])
 test(`the Admin tab is absent for ${name}`,async({page})=>{
  const s=await setup(page,opts);
  await expect(nav(page).getByRole('button',{name:'Connect',exact:true})).toBeVisible();
  await expect(nav(page).getByRole('button',{name:'Admin',exact:true})).toHaveCount(0);
  expect(s.calls).toEqual([]);
 });

test('lists developers, marking token-only accounts, and only token-only ones can be reissued',async({page})=>{
 await setup(page);await open(page);
 const list=page.getByRole('region',{name:'Developers in this product'});
 await expect(list.locator('li')).toHaveCount(2);
 const samRow=list.locator('li',{hasText:'sam'});const raviRow=list.locator('li',{hasText:'ravi'});
 await expect(samRow).toContainText('Token only (no sign-in)');await expect(samRow).toContainText('never used');
 await expect(samRow.getByRole('button',{name:'Reissue token for sam'})).toBeVisible();
 await expect(raviRow).toContainText('Signs in with the identity provider');await expect(raviRow).toContainText('reviewer');
 await expect(raviRow.getByRole('button',{name:/^Reissue/})).toHaveCount(0);
 await expect(page.locator('main')).not.toContainText('wkpt_');
});

test('creating a developer sends the right request and shows a one-time message to hand over',async({page})=>{
 await page.context().grantPermissions(['clipboard-read','clipboard-write']);
 const s=await setup(page);await open(page);
 const create=page.getByRole('button',{name:'Create developer and token'});
 await expect(create).toBeDisabled();
 await page.getByLabel('Name',{exact:true}).fill('jo');await expect(create).toBeDisabled();
 await page.getByLabel('backend').check();await page.getByLabel('Can review cards').check();
 await page.getByLabel('Token valid for').selectOption('90');
 await expect(create).toBeEnabled();await create.click();

 const post=s.calls.find(c=>c.method==='POST')!;
 expect(post.path).toBe('/v1/admin/products/alpha/users');expect(post.csrf).toBe('csrf-9');
 expect(post.body).toEqual({user:'jo',services:['backend'],can_review:true,days:90});

 const share=page.getByRole('region',{name:'Share with developer'});
 await expect(share.getByRole('heading',{name:'Send this to jo'})).toBeVisible();
 await expect(share).toContainText('shown only once');
 // The message is built for the agent the developer will use; Claude Code is the default.
 const message=share.locator('.share-message code');
 await expect(share.getByRole('radio',{name:'Claude Code'})).toBeChecked();
 let shown=await message.innerText();
 for(const line of [`MCP URL: ${url}`,`export WK_TOKEN=${secret}`,`claude mcp add --transport http workspace-knowledge ${url} --header "Authorization: Bearer $WK_TOKEN"`,'save as .mcp.json','save as CLAUDE.md','knowledge.team_work','Check it works'])expect(shown).toContain(line);
 expect(shown).not.toContain('<your token>');
 // Choosing Cursor rebuilds it for Cursor: its files, its header form, and still the real token.
 await share.getByRole('radio',{name:'Cursor'}).check();
 shown=await message.innerText();
 for(const line of ['save as .cursor/mcp.json','save as .cursor/rules/workspace-knowledge.mdc','Bearer ${env:WK_TOKEN}',`export WK_TOKEN=${secret}`])expect(shown).toContain(line);
 expect(shown).not.toContain('save as .mcp.json');
 // The configuration inside it is valid JSON and holds no secret; the secret is only in the token lines.
 const jsonPart=shown.slice(shown.indexOf('{\n  "mcpServers"'),shown.indexOf('}\n}\n')+3);
 expect(JSON.parse(jsonPart)).toEqual({mcpServers:{'workspace-knowledge':{url,headers:{Authorization:'Bearer ${env:WK_TOKEN}'}}}});
 expect(jsonPart).not.toContain(secret);
 // One button copies exactly what is shown; the pieces copy separately.
 await copyButton(page,'message for jo').click();
 expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(shown);
 await copyButton(page,'token for jo').click();expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(secret);
 await copyButton(page,'MCP URL for jo').click();expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(url);
 await expect(page.locator('[role=status].sr-only')).toContainText('Copied');

 // The list now has jo; dismissing removes the secret from the page entirely.
 await expect(page.getByRole('region',{name:'Developers in this product'}).locator('li',{hasText:'jo'})).toBeVisible();
 await share.getByRole('button',{name:'I have sent it'}).click();
 await expect(share).toHaveCount(0);expect(await page.locator('main').innerHTML()).not.toContain(secret);
 // The form was cleared for the next developer.
 await expect(page.getByLabel('Name',{exact:true})).toHaveValue('');await expect(page.getByLabel('backend')).not.toBeChecked();
});

test('a taken name and a refused scope are explained',async({page})=>{
 await setup(page,{create:r=>r.fulfill({status:409,json:{error:'conflict'}})});await open(page);
 await page.getByLabel('Name',{exact:true}).fill('ravi');await page.getByLabel('frontend').check();
 await page.getByRole('button',{name:'Create developer and token'}).click();
 await expect(page.getByRole('alert')).toContainText('already used by another developer');
 await expect(page.getByRole('region',{name:'Share with developer'})).toHaveCount(0);
 await page.unroute('**/v1/admin/products/alpha/users**');
 await page.route('**/v1/admin/products/alpha/users**',r=>r.request().method()==='GET'?r.fulfill({json:{services:['backend','frontend'],users:[],mcp_url:url}}):r.fulfill({status:403,json:{error:'forbidden'}}));
 await page.getByRole('button',{name:'Create developer and token'}).click();
 await expect(page.getByRole('alert')).toContainText('within your admin scope');
});

test('reissuing replaces the token and shows the new one once',async({page})=>{
 const s=await setup(page);await open(page);
 await page.getByRole('button',{name:'Reissue token for sam'}).click();
 const call=s.calls.find(c=>c.path.endsWith('/users/sam/token'))!;
 expect(call.method).toBe('POST');expect(call.body).toEqual({days:30});expect(call.csrf).toBe('csrf-9');
 const share=page.getByRole('region',{name:'Share with developer'});
 await expect(share.getByRole('heading',{name:'Send this to sam'})).toBeVisible();
 await expect(share.locator('.share-message code')).toContainText(`export WK_TOKEN=${secret2}`);
 await expect(page.locator('[role=status].sr-only')).toContainText('New token for sam');
});

test('removing needs a confirmation and says what it does',async({page})=>{
 const s=await setup(page);await open(page);
 const row=page.getByRole('region',{name:'Developers in this product'}).locator('li',{hasText:'sam'});
 await row.getByRole('button',{name:'Remove sam'}).click();
 await expect(page.getByText('revokes their access and any token at once',{exact:false})).toBeVisible();
 expect(s.calls.some(c=>c.method==='DELETE')).toBe(false);
 await row.getByRole('button',{name:'Cancel'}).click();expect(s.calls.some(c=>c.method==='DELETE')).toBe(false);
 await row.getByRole('button',{name:'Remove sam'}).click();await row.getByRole('button',{name:'Confirm remove sam'}).click();
 const del=s.calls.find(c=>c.method==='DELETE')!;
 expect(del.path).toBe('/v1/admin/products/alpha/users/sam');expect(del.csrf).toBe('csrf-9');
 await expect(page.getByRole('region',{name:'Developers in this product'}).locator('li',{hasText:'sam'})).toHaveCount(0);
 await expect(page.locator('[role=status].sr-only')).toContainText('Removed sam.');
});

test('a token or address that does not have the expected shape is never rendered or put in a message',async({page})=>{
 await setup(page,{create:r=>r.fulfill({status:201,json:{user:'bad',product:'alpha',token:'wkpt_"><img src=x onerror=window.pwned=1>',id:'z',label:'x',created_ms:1,expires_ms:2,mcp_url:'https://wk.example/mcp/alpha"><script>window.pwned=2</script>'}})});await open(page);
 await page.getByLabel('Name',{exact:true}).fill('bad');await page.getByLabel('backend').check();
 await page.getByRole('button',{name:'Create developer and token'}).click();
 await expect(page.getByRole('alert')).toContainText('did not return a usable MCP address');
 await expect(page.locator('main img, main script')).toHaveCount(0);await expect(page.getByRole('button',{name:/^Copy/})).toHaveCount(0);
 await expect(page.locator('.share-message')).toHaveCount(0);
 expect(await page.locator('main').innerText()).not.toContain('onerror');
 expect(await page.evaluate(()=>(window as unknown as {pwned?:number}).pwned)).toBeUndefined();
});

test('fits a phone screen',async({page})=>{
 await page.setViewportSize({width:390,height:844});
 await setup(page);await open(page);
 await page.getByLabel('Name',{exact:true}).fill('mobile');await page.getByLabel('backend').check();
 await page.getByRole('button',{name:'Create developer and token'}).click();
 await expect(page.getByRole('region',{name:'Share with developer'})).toBeVisible();
 expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
 await page.screenshot({path:'test-results/admin-mobile.png',fullPage:true});
});
