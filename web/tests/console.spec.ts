import {test,expect} from '@playwright/test';
test('shows authentication failures without persisting credentials',async({page})=>{
 await page.route('**/v1/search',route=>route.fulfill({status:401,contentType:'application/json',body:'{"error":"unauthenticated"}'}));
 await page.goto('/');await page.getByLabel('Product ID').fill('demo');await page.getByLabel('Access token').fill('SYNTHETIC_TOKEN');await page.getByRole('button',{name:'Connect',exact:true}).click();
 await page.getByLabel('Search product knowledge').fill('limiter');await page.getByRole('button',{name:'Search',exact:true}).click();await expect(page.getByRole('alert')).toContainText('not authorized');
 expect(await page.evaluate(()=>JSON.stringify(localStorage))).not.toContain('SYNTHETIC_TOKEN');expect(await page.evaluate(()=>JSON.stringify(sessionStorage))).not.toContain('SYNTHETIC_TOKEN');
});
test('empty authorized search is an honest empty state',async({page})=>{
 await page.route('**/v1/search',route=>route.fulfill({json:{cards:[]}}));await page.goto('/');await page.getByLabel('Product ID').fill('demo');await page.getByLabel('Access token').fill('FAKE');await page.getByRole('button',{name:'Connect',exact:true}).click();await page.getByLabel('Search product knowledge').fill('limiter');await page.getByRole('button',{name:'Search',exact:true}).click();await expect(page.getByText('No matching knowledge is visible')).toBeVisible();
});
test('console remains usable on a narrow viewport',async({page})=>{
 await page.setViewportSize({width:390,height:844});await page.goto('/');
 await expect(page.getByRole('heading',{name:'Connect to a product'})).toBeVisible();
 expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
 await page.screenshot({path:'test-results/login-mobile.png',fullPage:true});
});

const summaries = ['First limiter','Replacement limiter'].map((title,i)=>({id:`00000000-0000-0000-0000-00000000000${i+1}`,revision:i+1,published:true,body:{schema_version:1,idempotency_key:`card-${i}`,policy_revision:1,title,intent:'Tenant limits',rationale:'Shared middleware',decisions:[],constraints:[],open_questions:[],service_ids:['backend'],evidence:[],lifecycle:'proposed',capture_completeness:'complete',base_revision:null}}));
async function connect(page:import('@playwright/test').Page){await page.goto('/');await page.getByLabel('Product ID').fill('demo');await page.getByLabel('Access token').fill('SYNTHETIC');await page.getByRole('button',{name:'Connect',exact:true}).click();}
test('reviewed supersession uses exact revisions and rejects stale choices',async({page})=>{
 await page.route('**/v1/policy?*',r=>r.fulfill({json:{policy:{can_review:true}}}));await page.route('**/v1/products/demo/graph',r=>r.fulfill({json:{nodes:[],edges:[]}}));await page.route('**/v1/search',r=>r.fulfill({json:{cards:summaries}}));
 let attempts=0;await page.route('**/v1/products/demo/relationships',r=>{expect(r.request().postDataJSON()).toEqual({source_card:summaries[1].id,target_card:summaries[0].id,source_revision:2,target_revision:1,kind:'supersedes'});return r.fulfill({status:++attempts===1?409:200,json:{linked:attempts>1}});});
 await connect(page);await page.getByRole('button',{name:'Relationships',exact:true}).click();await page.getByText('Add a reviewed relationship',{exact:true}).click();await page.getByLabel('Find related summaries').fill('limiter');
 for(let attempt=0;attempt<2;attempt++){
  await page.getByRole('button',{name:'Find summaries',exact:true}).click();await page.getByRole('combobox',{name:'Source summary',exact:true}).selectOption(summaries[1].id);await page.getByRole('combobox',{name:'Related summary',exact:true}).selectOption(summaries[0].id);await page.getByRole('combobox',{name:'Relationship',exact:true}).selectOption('supersedes');await expect(page.getByRole('button',{name:'Save relationship'})).toBeDisabled();await page.getByRole('checkbox').check();await page.getByRole('button',{name:'Save relationship'}).click();
  if(attempt===0){await expect(page.getByRole('alert')).toContainText('Reload the latest revision');await expect(page.getByRole('button',{name:'Save relationship'})).toBeDisabled();}
 }
 await expect(page.getByText('Relationship saved.',{exact:true})).toBeVisible();await page.setViewportSize({width:390,height:844});expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);await page.getByRole('heading',{name:'Relationships',exact:true}).click();await page.evaluate(()=>window.scrollTo(0,0));await page.screenshot({path:'test-results/relationships-mobile.png',fullPage:true});
});
test('trusted evidence freshness is separate from reported references',async({page})=>{
 await page.route('**/v1/search',r=>r.fulfill({json:{cards:[summaries[0]]}}));await page.route(`**/v1/products/demo/cards/${summaries[0].id}`,r=>r.fulfill({json:{card:summaries[0]}}));await page.route('**/evidence',r=>r.fulfill({json:{evidence:[{provider:'ci',reference:'https://example.org/build/1',code_revision:'abcdef123',kind:'tests_verified',card_revision:1,stale:true}]}}));
 await connect(page);await page.getByLabel('Search product knowledge').fill('limiter');await page.getByRole('button',{name:'Search',exact:true}).click();await page.getByRole('button').filter({hasText:'First limiter'}).click();await expect(page.getByRole('region',{name:'Trusted evidence'})).toContainText('tests verified · Stale');await expect(page.getByRole('heading',{name:'Author-reported references'})).toBeVisible();
});
