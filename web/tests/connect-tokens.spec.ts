import {test,expect,type Page,type Route} from '@playwright/test';
const url='https://wk.example/mcp/alpha';
const secret=`wkpt_${'ab12'.repeat(16)}`;
type Tok={id:string;product:string;label:string;created_ms:number;expires_ms:number;last_used_ms:number|null};
const day=86_400_000;
const existing:Tok={id:'11111111-1111-4111-8111-111111111111',product:'alpha',label:'Old laptop',created_ms:Date.UTC(2026,8,1),expires_ms:Date.UTC(2026,10,1),last_used_ms:Date.UTC(2026,8,20)};
const otherProduct:Tok={...existing,id:'22222222-2222-4222-8222-222222222222',product:'beta',label:'Beta only'};

async function setup(page:Page,opts:{settings?:{max_days:number}|null|'omitted';tokens?:Tok[];create?:(route:Route)=>Promise<void>|void}={}){
 const state={tokens:opts.tokens??[existing,otherProduct],requests:[] as {method:string;path:string;csrf:string|undefined;body:unknown}[]};
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 const me={user:'ravi',org:'team',csrf_token:'csrf-1',products:[{product:'alpha',services:['backend'],can_review:false,policy_revision:1,mcp_url:url}]};
 await page.route('**/v1/me',r=>r.fulfill({json:opts.settings==='omitted'?me:{...me,personal_tokens:opts.settings===undefined?{max_days:90}:opts.settings}}));
 await page.route('**/personal',r=>r.fulfill({json:{summaries:[],next_cursor:null}}));
 await page.route('**/v1/tokens**',async r=>{
  const req=r.request();const path=new URL(req.url()).pathname;
  state.requests.push({method:req.method(),path,csrf:req.headers()['x-csrf-token'],body:req.postDataJSON()});
  if(req.method()==='GET')return r.fulfill({json:{tokens:state.tokens}});
  if(req.method()==='DELETE'){state.tokens=state.tokens.filter(t=>!path.endsWith(t.id));return r.fulfill({json:{revoked:true}});}
  if(opts.create)return opts.create(r);
  const body=req.postDataJSON();const made:Tok={id:'33333333-3333-4333-8333-333333333333',product:body.product,label:body.label,created_ms:Date.now(),expires_ms:Date.now()+body.days*day,last_used_ms:null};
  state.tokens=[made,...state.tokens];return r.fulfill({status:201,json:{...made,token:secret}});
 });
 await page.goto('/');
 await page.getByRole('button',{name:'Connect',exact:true}).click();
 return state;
}
const section=(page:Page)=>page.getByRole('region',{name:'Product alpha'}).locator('.tokens');

for(const [name,settings] of [['disabled on the server',null],['an older server that omits the setting','omitted']] as const)
 test(`the token section is absent for ${name}`,async({page})=>{
  const s=await setup(page,{settings});
  await expect(page.getByRole('region',{name:'Product alpha'}).getByLabel('MCP server URL')).toBeVisible();
  await expect(page.getByRole('heading',{name:'Personal access token'})).toHaveCount(0);
  expect(s.requests).toEqual([]);
 });

test('lists only this product’s tokens and creates one that is shown once with ready snippets',async({page})=>{
 await page.context().grantPermissions(['clipboard-read','clipboard-write']);
 const s=await setup(page);const tokens=section(page);
 await expect(tokens.getByRole('heading',{name:'Personal access token'})).toBeVisible();
 const names=tokens.locator('.token-list strong');await expect(names).toHaveText(['Old laptop']);
 await expect(tokens.getByText('Never used')).toHaveCount(0);await expect(tokens).toContainText('Last used');
 await expect(page.locator('main')).not.toContainText(secret);

 await tokens.getByLabel('Name').fill('  Cursor on my Mac ');
 await expect(tokens.getByLabel('Valid for')).toHaveValue('30');
 await tokens.getByLabel('Valid for').selectOption('90');
 await tokens.getByRole('button',{name:'Create token'}).click();

 const post=s.requests.find(r=>r.method==='POST')!;
 expect(post.path).toBe('/v1/tokens');expect(post.csrf).toBe('csrf-1');
 expect(post.body).toEqual({product:'alpha',label:'Cursor on my Mac',days:90});

 const once=tokens.getByRole('region',{name:'New token'});
 await expect(once).toContainText('It will not be shown again.');
 await expect(once.getByLabel('Token',{exact:true})).toHaveValue(secret);await expect(once.getByLabel('Token',{exact:true})).toHaveJSProperty('readOnly',true);
 await expect(page.locator('[role=status].sr-only')).toContainText('Token created.');
 const copied=async(label:string)=>{await page.getByRole('button',{name:new RegExp(`^(Copy|Copied) ${label}$`)}).click();return page.evaluate(()=>navigator.clipboard.readText());};
 expect(await copied('token for alpha')).toBe(secret);
 expect(await copied('Environment variable for alpha')).toBe(`export WK_TOKEN=${secret}`);
 expect(await copied('Claude Code command with token for alpha')).toBe(`claude mcp add --transport http workspace-knowledge ${url} --header "Authorization: Bearer $WK_TOKEN"`);
 const cursor=JSON.parse(await copied('Cursor configuration with token for alpha'));
 expect(cursor).toEqual({mcpServers:{'workspace-knowledge':{url,headers:{Authorization:'Bearer ${env:WK_TOKEN}'}}}});
 // The committed configuration never contains the secret itself.
 expect(JSON.stringify(cursor)).not.toContain(secret);

 // The list gained the new token without the secret, and dismissing removes the secret from the page.
 await expect(tokens.locator('.token-list li')).toHaveCount(2);await expect(tokens.locator('.token-list')).toContainText('Cursor on my Mac');
 await expect(tokens.locator('.token-list')).not.toContainText(secret);
 await once.getByRole('button',{name:'I have saved the token'}).click();
 await expect(once).toHaveCount(0);expect(await page.locator('main').innerHTML()).not.toContain(secret);
});

test('revoking removes the token and sends the session’s CSRF token',async({page})=>{
 const s=await setup(page);const tokens=section(page);
 await tokens.getByRole('button',{name:'Revoke token Old laptop'}).click();
 await expect(tokens.locator('.token-list')).toHaveCount(0);await expect(tokens.getByText('No active tokens.')).toBeVisible();
 const del=s.requests.find(r=>r.method==='DELETE')!;
 expect(del.path).toBe(`/v1/tokens/${existing.id}`);expect(del.csrf).toBe('csrf-1');
 await expect(page.locator('[role=status].sr-only')).toContainText('Revoked token Old laptop.');
});

test('the lifetime choices follow the server’s limit',async({page})=>{
 await setup(page,{settings:{max_days:14}});
 const options=await section(page).getByLabel('Valid for').locator('option').allTextContents();
 expect(options).toEqual(['7 days','14 days']);
 await expect(section(page).getByLabel('Valid for')).toHaveValue('14');
});

test('the per-user limit and other failures are explained, not hidden',async({page})=>{
 await setup(page,{create:r=>r.fulfill({status:409,json:{error:'conflict'}})});const tokens=section(page);
 await tokens.getByLabel('Name').fill('One too many');await tokens.getByRole('button',{name:'Create token'}).click();
 await expect(tokens.getByRole('alert')).toContainText('maximum number of active tokens');
 await expect(tokens.getByRole('region',{name:'New token'})).toHaveCount(0);
});

test('a token that does not have the server’s shape is never shown or put in a snippet',async({page})=>{
 await setup(page,{create:r=>r.fulfill({status:201,json:{id:'44444444-4444-4444-8444-444444444444',product:'alpha',label:'x',created_ms:1,expires_ms:2,last_used_ms:null,token:'wkpt_"><img src=x onerror=window.pwned=1>'}})});const tokens=section(page);
 await tokens.getByLabel('Name').fill('x');await tokens.getByRole('button',{name:'Create token'}).click();
 await expect(tokens.getByRole('alert')).toContainText('could not be created');
 await expect(page.locator('main img')).toHaveCount(0);expect(await page.locator('main').innerText()).not.toContain('onerror');
 expect(await page.evaluate(()=>(window as unknown as {pwned?:number}).pwned)).toBeUndefined();
});

test('the name is required and the section is usable from the keyboard on a phone',async({page})=>{
 await page.setViewportSize({width:390,height:844});
 await setup(page);const tokens=section(page);
 await expect(tokens.getByRole('button',{name:'Create token'})).toBeDisabled();
 await tokens.getByLabel('Name').focus();await page.keyboard.type('Phone test');await page.keyboard.press('Tab');await page.keyboard.press('Tab');
 await expect(tokens.getByRole('button',{name:'Create token'})).toBeFocused();await page.keyboard.press('Enter');
 await expect(tokens.getByRole('region',{name:'New token'})).toBeVisible();
 expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
 await tokens.scrollIntoViewIfNeeded();await page.screenshot({path:'test-results/connect-tokens-mobile.png',fullPage:true});
});
