import {test,expect,type Page,type Route} from '@playwright/test';
import {readFileSync} from 'node:fs';
type File={path:string;kind:string;format:string;content:string;merge:string};
type Client={id:string;name:string;summary:string;steps:string[];files:File[];command:string|null;verify:string;notes:string[];docs:string};
const fixture=(name:string)=>JSON.parse(readFileSync(new URL(`./fixtures/${name}`,import.meta.url),'utf8'));
const base=fixture('agent-setup.json') as {clients:Client[];rules:string;mcp_url:string};
const names=['Claude Code','Claude Desktop','Codex','Cursor','VS Code (GitHub Copilot)'];

async function setup(page:Page,opts:{setup?:(route:Route)=>Promise<void>|void}={}){
 const calls:string[]=[];
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({json:{user:'ravi',org:'team',csrf_token:'c',personal_tokens:{max_days:90},products:[{product:'alpha',services:['backend'],can_review:false,policy_revision:1,mcp_url:base.mcp_url,is_admin:false}]}}));
 await page.route('**/personal',r=>r.fulfill({json:{summaries:[],next_cursor:null}}));
 await page.route('**/v1/tokens**',r=>r.fulfill({json:{tokens:[]}}));
 await page.route('**/v1/products/alpha/agent-setup**',async r=>{
  const url=r.request().url();calls.push(new URL(url).search);
  if(opts.setup)return opts.setup(r);
  return r.fulfill({json:fixture(url.includes('auto_share=true')?'agent-setup-auto.json':'agent-setup.json')});
 });
 await page.goto('/');
 await page.getByRole('button',{name:'Connect',exact:true}).click();
 return calls;
}
const panel=(page:Page)=>page.getByRole('region',{name:'Product alpha'}).locator('.agent-setup');
const pick=async(page:Page,name:string)=>{await panel(page).getByRole('radio',{name,exact:true}).check();};
const copied=async(page:Page,label:string)=>{await page.getByRole('button',{name:new RegExp(`^(Copy|Copied) ${label.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')}$`)}).click();return page.evaluate(()=>navigator.clipboard.readText());};

test.beforeEach(async({context})=>{await context.grantPermissions(['clipboard-read','clipboard-write']);});

test('offers all five agents and starts on Claude Code',async({page})=>{
 await setup(page);const p=panel(page);
 await expect(p.getByRole('heading',{name:'Set up your coding agent',level:3})).toBeVisible();
 const radios=p.getByRole('radio');
 await expect(radios).toHaveCount(5);
 await expect(p.locator('.client-option')).toHaveText(names);
 await expect(p.getByRole('radio',{name:'Claude Code'})).toBeChecked();
});

test('each agent shows exactly its own files, steps, command and check, and copies them verbatim',async({page})=>{
 await setup(page);const p=panel(page);
 for(const client of base.clients){
  await pick(page,client.name);
  await expect(p.locator('li').first()).toBeVisible();
  for(const step of client.steps)await expect(p).toContainText(step);
  const shown=await p.locator('.snippet pre code').allInnerTexts();
  const expected=[...client.files.map(f=>f.content),...(client.command?[client.command]:[])];
  expect(shown.map(t=>t.trim())).toEqual(expected.map(t=>t.trim()));
  for(const f of client.files){
   await expect(p.locator('h4',{hasText:f.path})).toBeVisible();
   expect(await copied(page,`${f.path} for ${client.name}`)).toBe(f.content);
  }
  if(client.command)expect(await copied(page,`${client.name} command`)).toBe(client.command);
  await expect(p).toContainText(client.verify);
 }
});

test('the configuration files are valid and never contain a token',async({page})=>{
 await setup(page);const p=panel(page);
 for(const client of base.clients){
  await pick(page,client.name);
  const text=await p.innerText();
  expect(text).not.toContain('wkpt_');
  for(const f of client.files.filter(f=>f.format==='json'))expect(()=>JSON.parse(f.content)).not.toThrow();
 }
 await pick(page,'Codex');
 await expect(p).toContainText('bearer_token_env_var = "WK_TOKEN"');
 await pick(page,'VS Code (GitHub Copilot)');
 await expect(p).toContainText('${input:wk-token}');await expect(p).toContainText('"password": true');
 await pick(page,'Claude Desktop');
 await expect(p).toContainText('"proxy"');await expect(p).toContainText('PASTE_YOUR_TOKEN_HERE');
});

test('asking for automatic sharing fetches the other instructions',async({page})=>{
 const calls=await setup(page);const p=panel(page);
 await expect(p).toContainText('Do not share unless I ask');
 await p.getByLabel('Tell the agent to share finished work automatically',{exact:false}).check();
 await expect(p).toContainText('Sharing is on for this project');await expect(p).not.toContainText('Do not share unless I ask');
 // (React's development mode mounts twice, so the plain request may appear twice; the point is the order.)
 expect(calls[0]).toBe('');expect(calls.filter(c=>c==='?auto_share=true')).toHaveLength(1);expect(calls.at(-1)).toBe('?auto_share=true');
 await p.getByLabel('Tell the agent to share finished work automatically',{exact:false}).uncheck();
 await expect(p).toContainText('Do not share unless I ask');
});

test('a failed or unusable setup is explained or hidden, never half shown',async({page})=>{
 await setup(page,{setup:r=>r.fulfill({status:500,json:{error:'x'}})});
 await expect(panel(page).getByText('could not be loaded')).toBeVisible();
 await expect(panel(page).getByRole('radio')).toHaveCount(0);
 await page.unrouteAll();
 await setup(page,{setup:r=>r.fulfill({json:{...base,mcp_url:null,clients:[]}})});
 await expect(page.getByRole('heading',{name:'Set up your coding agent'})).toHaveCount(0);
});

test('a documentation address that is not plain https is never made a link',async({page})=>{
 const hostile=base.clients.map((c:Client,i:number)=>i===0?{...c,docs:'javascript:alert(1)'}:c);
 await setup(page,{setup:r=>r.fulfill({json:{...base,clients:hostile}})});
 const p=panel(page);
 await expect(p.getByRole('link')).toHaveCount(0);
 expect(await page.evaluate(()=>(window as unknown as {pwned?:number}).pwned)).toBeUndefined();
 await pick(page,'Claude Desktop');
 const link=p.getByRole('link',{name:'Claude Desktop documentation'});
 await expect(link).toHaveAttribute('href',/^https:\/\//);await expect(link).toHaveAttribute('rel','noopener noreferrer');await expect(link).toHaveAttribute('target','_blank');
});

test('fits a phone screen',async({page})=>{
 await page.setViewportSize({width:390,height:844});
 await setup(page);await pick(page,'Codex');
 expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
 await panel(page).scrollIntoViewIfNeeded();
 await page.screenshot({path:'test-results/agent-setup-mobile.png',fullPage:true});
});
