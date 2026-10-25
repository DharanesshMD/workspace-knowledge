import {test,expect,type Page} from '@playwright/test';
const alphaUrl='https://wk.example/mcp/alpha';
const products=(extra:Record<string,unknown>={})=>[
 {product:'alpha',services:['backend'],can_review:false,policy_revision:1,mcp_url:alphaUrl},
 {product:'beta',services:['frontend'],can_review:false,policy_revision:2,mcp_url:null,...extra},
];
async function signIn(page:Page,list=products()){
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({json:{user:'alice',org:'team',csrf_token:'csrf',products:list}}));
 await page.route('**/personal',r=>r.fulfill({json:{summaries:[],next_cursor:null}}));
 await page.goto('/');
}
// The button's name changes while it confirms, so find it by label under any of its three names.
const copyButton=(page:Page,label:string)=>page.getByRole('button',{name:new RegExp(`^(Copy|Copied|Copy failed) ${label}$`)});
const open=async(page:Page)=>{await page.getByRole('button',{name:'Connect',exact:true}).click();await expect(page.getByRole('heading',{name:'Connect',level:1})).toBeVisible();};
const snippets:[string,string][]=[
 ['MCP URL for alpha',alphaUrl],
 ['Claude connector for alpha',`Name: Workspace Knowledge\nMCP server URL: ${alphaUrl}`],
 ['Claude Code command for alpha',`claude mcp add --transport http workspace-knowledge ${alphaUrl}`],
 ['Cursor configuration for alpha',`{\n  "mcpServers": {\n    "workspace-knowledge": {\n      "url": "${alphaUrl}"\n    }\n  }\n}`],
 ['VS Code configuration for alpha',`{\n  "servers": {\n    "workspace-knowledge": {\n      "type": "http",\n      "url": "${alphaUrl}"\n    }\n  }\n}`],
];
test.describe('with clipboard access',()=>{
 test.beforeEach(async({context})=>{await context.grantPermissions(['clipboard-read','clipboard-write']);});

 test('lists each product with its address, setup snippets and guidance',async({page})=>{
  await signIn(page);await open(page);
  await expect(page.getByText('Fetch your team’s shared context')).toBeVisible();
  await expect(page.getByText('It never sees your conversation.',{exact:false})).toBeVisible();
  const alpha=page.getByRole('region',{name:'Product alpha'});
  await expect(alpha.getByLabel('MCP server URL')).toHaveValue(alphaUrl);
  await expect(alpha.getByLabel('MCP server URL')).toHaveJSProperty('readOnly',true);
  for(const title of ['Claude connector','Claude Code command','Cursor configuration','VS Code configuration'])await expect(alpha.getByRole('heading',{name:title,level:3})).toBeVisible();
  await expect(alpha).toContainText('Settings, Connectors, Add custom connector');
  await expect(alpha).toContainText('connected from Anthropic’s side');await expect(alpha).toContainText('not been tested here');
  await expect(alpha).toContainText('.cursor/mcp.json');await expect(alpha).toContainText('.vscode/mcp.json');await expect(alpha).toContainText('above 5 minutes');
  await expect(page.getByText('You will be asked to sign in once in your browser the first time the agent connects.',{exact:false})).toBeVisible();
  await expect(page.getByText('install the local bridge',{exact:false})).toContainText('wk-mcp');
  expect(await page.locator('main').innerHTML()).not.toContain('<script');
 });

 test('every copy button puts exactly its text on the clipboard and says so',async({page})=>{
  await signIn(page);await open(page);
    for(const [label,expected] of snippets){
   const button=copyButton(page,label);
   await button.click();
   await expect(button).toHaveText(`Copied ${label}`);
   expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(expected);
   await expect(page.locator('[role=status].sr-only')).toHaveText(`Copied ${label}.`);
  }
  // The JSON snippets are valid configuration files.
  expect(JSON.parse(snippets[3][1])).toEqual({mcpServers:{'workspace-knowledge':{url:alphaUrl}}});
  expect(JSON.parse(snippets[4][1])).toEqual({servers:{'workspace-knowledge':{type:'http',url:alphaUrl}}});
 });

 test('the confirmation reverts and a second copy is announced again',async({page})=>{
  await signIn(page);await open(page);
  const button=copyButton(page,'Claude Code command for alpha');
  await button.click();await expect(button).toHaveText('Copied Claude Code command for alpha');await expect(button).toHaveText('Copy Claude Code command for alpha',{timeout:6000});
  const live=page.locator('[role=status].sr-only');await button.click();await expect(live).toHaveText('Copied Claude Code command for alpha.');
 });

 test('several products keep separate controls and a missing address explains itself',async({page})=>{
  await signIn(page);await open(page);
  const beta=page.getByRole('region',{name:'Product beta'});
  await expect(beta.getByText('Your administrator has not configured the MCP address yet (WK_MCP_RESOURCE).')).toBeVisible();
  await expect(beta.getByRole('button')).toHaveCount(0);await expect(beta.getByRole('textbox')).toHaveCount(0);
  await expect(page.getByRole('button',{name:/^Copy .* for beta$/})).toHaveCount(0);
 });

 test('two configured products each copy their own address',async({page})=>{
  await signIn(page,products({mcp_url:'https://wk.example/mcp/beta'}));await open(page);
  await page.getByRole('button',{name:'Copy Claude Code command for beta'}).click();
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe('claude mcp add --transport http workspace-knowledge https://wk.example/mcp/beta');
  await page.getByRole('button',{name:'Copy Cursor configuration for alpha'}).click();
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toContain(alphaUrl);
 });

 test('is usable from the keyboard',async({page})=>{
  await signIn(page);
  const nav=page.getByRole('navigation',{name:'Main navigation'}).getByRole('button',{name:'Connect',exact:true});
  await nav.focus();await page.keyboard.press('Enter');
  await expect(page.getByRole('heading',{name:'Connect',level:1})).toBeVisible();await expect(nav).toHaveAttribute('aria-current','page');
  const field=page.getByRole('region',{name:'Product alpha'}).getByLabel('MCP server URL');
  await field.focus();await page.keyboard.press('Tab');
  const copy=copyButton(page,'MCP URL for alpha');await expect(copy).toBeFocused();
  await page.keyboard.press('Enter');await expect(copy).toContainText('Copied');
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(alphaUrl);
  await page.keyboard.press('Tab');const next=copyButton(page,'Claude connector for alpha');await expect(next).toBeFocused();
  await page.keyboard.press('Space');await expect(next).toContainText('Copied');
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(snippets[1][1]);
 });

 test('fits a phone screen and a desktop screen',async({page})=>{
  await page.setViewportSize({width:390,height:844});await signIn(page,products({mcp_url:'https://knowledge.very-long-team-name.example.com/mcp/beta'}));await open(page);
  await expect(page.getByRole('button',{name:'Copy Cursor configuration for alpha'})).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.evaluate(()=>{(document.activeElement as HTMLElement|null)?.blur();window.scrollTo(0,0);});
  await page.screenshot({path:'test-results/connect-mobile.png',fullPage:true});
  await page.setViewportSize({width:1440,height:1000});expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(1440);
  await page.screenshot({path:'test-results/connect-desktop.png',fullPage:true});
 });
});

test('an address that is not a plain MCP address is never rendered or put in a snippet',async({page})=>{
 const hostile='https://wk.example/mcp/alpha"><img src=x onerror=window.pwned=1>';
 await signIn(page,[{product:'alpha',services:['backend'],can_review:false,policy_revision:1,mcp_url:hostile},{product:'beta',services:['x'],can_review:false,policy_revision:1,mcp_url:'http://wk.example/mcp/beta'}]);await open(page);
 await expect(page.getByText('cannot be used',{exact:false})).toHaveCount(2);
 await expect(page.getByRole('button',{name:/^Copy/})).toHaveCount(0);await expect(page.locator('main img')).toHaveCount(0);
 expect(await page.locator('main').innerText()).not.toContain('onerror');expect(await page.evaluate(()=>(window as unknown as {pwned?:number}).pwned)).toBeUndefined();
});

test('older servers that omit the address show the administrator message',async({page})=>{
 await signIn(page,[{product:'alpha',services:['backend'],can_review:false,policy_revision:1}]);await open(page);
 await expect(page.getByText('Your administrator has not configured the MCP address yet (WK_MCP_RESOURCE).')).toBeVisible();
});

test('copying falls back to selection copy when the clipboard API is unavailable',async({page})=>{
 await page.addInitScript(()=>{
  Object.defineProperty(navigator,'clipboard',{value:undefined,configurable:true});
  (window as unknown as {copied:string[]}).copied=[];
  document.execCommand=(command:string)=>{const a=document.activeElement as HTMLTextAreaElement;(window as unknown as {copied:string[]}).copied.push(a.value.slice(a.selectionStart,a.selectionEnd));return command==='copy';};
 });
 await signIn(page);await open(page);
 const button=copyButton(page,'Claude Code command for alpha');await button.focus();await button.click();
 await expect(button).toContainText('Copied');await expect(button).toBeFocused();
 expect(await page.evaluate(()=>(window as unknown as {copied:string[]}).copied)).toEqual([snippets[2][1]]);
 await expect(page.locator('main textarea')).toHaveCount(0);
});

test('a copy that cannot happen is reported, not claimed',async({page})=>{
 await page.addInitScript(()=>{
  Object.defineProperty(navigator,'clipboard',{value:{writeText:()=>Promise.reject(new Error('denied'))},configurable:true});
  document.execCommand=()=>false;
 });
 await signIn(page);await open(page);
 const button=copyButton(page,'MCP URL for alpha');await button.click();
 await expect(button).toContainText('Copy failed');await expect(button).not.toContainText('Copied');
 await expect(page.locator('[role=status].sr-only')).toContainText('Could not copy MCP URL for alpha');
 await expect(page.getByLabel('MCP server URL').first()).toHaveValue(alphaUrl);
});
