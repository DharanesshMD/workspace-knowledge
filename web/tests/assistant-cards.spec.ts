import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:false,development:true}}));});
const body=(over:object)=>({schema_version:1,idempotency_key:'k',policy_revision:1,title:'Cap retries',intent:'Avoid repeat charges',rationale:'Gateway charges twice',decisions:[],constraints:[],open_questions:[],service_ids:['backend'],evidence:[],lifecycle:'proposed',capture_completeness:'partial',base_revision:null,...over});
const queue=[
 {id:'00000000-0000-0000-0000-000000000001',revision:1,published:false,body:body({title:'Assistant card',provenance:'assistant_written'})},
 {id:'00000000-0000-0000-0000-000000000002',revision:1,published:false,body:body({title:'[auto-drafted] Rule card'})},
 {id:'00000000-0000-0000-0000-000000000003',revision:1,published:false,body:body({title:'Truncated card',capture_completeness:'truncated'})},
 {id:'00000000-0000-0000-0000-000000000004',revision:1,published:true,body:body({title:'Live assistant summary',provenance:'assistant_written'})},
];
async function openQueue(page:import('@playwright/test').Page){
 await page.route('**/v1/policy?*',r=>r.fulfill({json:{policy:{can_review:true}}}));
 await page.route('**/v1/products/demo/review',r=>r.fulfill({json:{cards:queue}}));
 await page.route('**/v1/products/demo/cards/*/evidence',r=>r.fulfill({json:{evidence:[]}}));
 await page.route('**/v1/products/demo/cards/*',r=>r.fulfill({json:{card:queue.find(c=>r.request().url().endsWith(c.id))}}));
 await page.goto('/');await page.getByLabel('Product ID').fill('demo');await page.getByLabel('Access token').fill('SYNTHETIC');
 await page.getByRole('button',{name:'Connect',exact:true}).click();await page.getByRole('button',{name:'Review queue',exact:true}).click();
}
test('an assistant-written card can be published and says who wrote it',async({page})=>{
 await openQueue(page);
 const row=page.getByRole('button').filter({hasText:'Assistant card'});
 await expect(row).toContainText('Assistant-written');
 await expect(row).not.toContainText(/partial capture/i);
 await row.click();
 await expect(page.getByText('Written by the author’s assistant; submitted after the author’s review')).toBeVisible();
 await expect(page.getByRole('note')).toContainText('Publishing makes it searchable');
 await expect(page.getByRole('note')).toHaveAttribute('id','assistant-note');
 await expect(page.getByText('Capture coverage')).toHaveCount(0);
 const publish=page.getByRole('button',{name:'Publish summary'});
 await expect(publish).toBeEnabled();
 await expect(publish).toHaveAttribute('aria-describedby','assistant-note');
});
test('rule drafts and truncated cards still cannot be published',async({page})=>{
 await openQueue(page);
 for(const title of ['Rule card','Truncated card']){
  await page.getByRole('button').filter({hasText:title}).click();
  await expect(page.getByRole('button',{name:'Publish summary'})).toBeDisabled();
  await expect(page.getByRole('note')).toHaveCount(0);
  await expect(page.getByText('Capture coverage')).toBeVisible();
  await expect(page.locator('dd.warning')).toBeVisible();
 }
});
test('an already published assistant card shows its label but no publish notice',async({page})=>{
 await openQueue(page);
 const row=page.getByRole('button').filter({hasText:'Live assistant summary'});
 await expect(row).toContainText('Assistant-written');
 await row.click();
 await expect(page.getByText('Written by the author’s assistant; submitted after the author’s review')).toBeVisible();
 await expect(page.getByRole('note')).toHaveCount(0);
 await expect(page.getByRole('button',{name:'Publish summary'})).toHaveCount(0);
});
test('correcting an assistant-written card keeps its provenance and base revision',async({page})=>{
 let posted:any=null;
 await page.route('**/v1/cards/*/correct',r=>{posted=r.request().postDataJSON();return r.fulfill({json:{}});});
 await openQueue(page);
 await page.getByRole('button').filter({hasText:'Assistant card'}).click();
 await page.getByRole('button',{name:'Correct summary'}).click();
 await page.getByLabel('Intent').fill('Avoid repeat charges on retry');
 await page.getByRole('button',{name:'Save correction'}).click();
 await expect.poll(()=>posted).not.toBeNull();
 expect(posted.card.provenance).toBe('assistant_written');
 expect(posted.card.base_revision).toBe(1);
 expect(posted.card.intent).toBe('Avoid repeat charges on retry');
});
