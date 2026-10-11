import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{await page.route('**/products/*/personal*',r=>r.fulfill({json:{summaries:[],next_cursor:null}}));});

test('hosted change brief is source-backed and requires browser acknowledgement',async({page})=>{
 await page.setViewportSize({width:1180,height:900});
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({json:{user:'bob',org:'team',csrf_token:'csrf-test',products:[{product:'alpha',services:['backend'],can_review:false,policy_revision:3},{product:'beta',services:['frontend'],can_review:false,policy_revision:1}]}}));
 await page.route('**/preflights/brief-123',r=>r.fulfill({json:{brief:{schema_version:1,repository_id:'repo',base_revision:'a'.repeat(40),targets:[{path:'backend/retry.rs',start:null,end:null}],decision:'needs_review',reasons:['Missing dependency rationale'],gaps:['dependency_coverage_incomplete'],coverage:'reviewed records only',observation_assurance:'local_git_snapshot',items:[{role:'contributor',display_name:'Alice <script>alert(1)</script>',text:'Confirmed retry invariant',matched_by:'confirmed_contribution',sources:[{kind:'contribution',id:'c1',revision:2,code_revision:'a'.repeat(40),assurance:'reviewed',freshness:'current',observed_at_ms:1}]}],truncated:false,brief_id:'brief-123',expires_at_ms:Date.now()+60000},operation_class:'edit',acknowledged:false,consumed:false}}));
 let ackHeaders:Record<string,string>|undefined;
 await page.route('**/preflights/brief-123/acknowledge',r=>{ackHeaders=r.request().headers();return r.fulfill({json:{acknowledged:true}});});
 await page.goto('/?brief=brief-123');
 await expect(page.getByRole('heading',{name:'Before changing this code'})).toBeVisible();
 await expect(page.getByText('Alice <script>alert(1)</script>')).toBeVisible();
 await expect(page.locator('.code-context script')).toHaveCount(0);
 await expect(page.getByText('Dependency coverage is incomplete.')).toBeVisible();
 await page.screenshot({path:'test-results/code-context-desktop.png',fullPage:true});
 const acknowledge=page.getByRole('button',{name:'Acknowledge this change'});
 await expect(acknowledge).toBeDisabled();
 await page.getByRole('checkbox',{name:/I reviewed the exact targets/}).check();
 await expect(acknowledge).toBeEnabled();
 await acknowledge.click();
 await expect(page.getByText('Acknowledged for this operation only.')).toBeVisible();
 await expect(acknowledge).toBeDisabled();
 expect(ackHeaders?.['x-csrf-token']).toBe('csrf-test');
 expect(ackHeaders?.authorization).toBeUndefined();
 await page.setViewportSize({width:390,height:844});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
 await page.screenshot({path:'test-results/code-context-mobile.png',fullPage:true});
});

test('expired brief remains visible as reference but cannot be acknowledged',async({page})=>{
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({json:{user:'bob',org:'team',csrf_token:'csrf',products:[{product:'alpha',services:['backend'],can_review:false,policy_revision:1}]}}));
 await page.route('**/preflights/brief-expired',r=>r.fulfill({json:{brief:{schema_version:1,repository_id:'repo',base_revision:'a'.repeat(40),targets:[{path:'backend/retry.rs',start:null,end:null}],decision:'needs_review',reasons:[],gaps:[],coverage:'reviewed records only',observation_assurance:'local_git_snapshot',items:[],truncated:false,brief_id:'brief-expired',expires_at_ms:Date.now()-1000},operation_class:'edit',acknowledged:false,consumed:false}}));
 await page.goto('/?brief=brief-expired');
 await expect(page.getByText('This brief expired. Return to the agent for a fresh review.')).toBeVisible();
 await expect(page.getByRole('button',{name:'Acknowledge this change'})).toBeDisabled();
});

test('changing product while a brief request is pending cannot enable its acknowledgement',async({page})=>{
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({json:{user:'bob',org:'team',csrf_token:'csrf',products:[{product:'alpha',services:['backend'],can_review:false,policy_revision:1},{product:'beta',services:['frontend'],can_review:false,policy_revision:1}]}}));
 await page.route('**/preflights/brief-123',async r=>{await new Promise(resolve=>setTimeout(resolve,250));await r.fulfill({json:{brief:{schema_version:1,repository_id:'repo',base_revision:'a'.repeat(40),targets:[{path:'backend/retry.rs',start:null,end:null}],decision:'needs_review',reasons:[],gaps:[],coverage:'reviewed records only',observation_assurance:'local_git_snapshot',items:[],truncated:false,brief_id:'brief-123',expires_at_ms:Date.now()+60000},operation_class:'edit',acknowledged:false,consumed:false}});});
 await page.goto('/?brief=brief-123');
 await page.getByLabel('Current product').selectOption('beta');
 await expect(page.getByRole('button',{name:'Acknowledge this change'})).toHaveCount(0);
});
