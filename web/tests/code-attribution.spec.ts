import {test,expect} from '@playwright/test';

test('developer profile is opt-in and only the owner confirms their provenance',async({page})=>{
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({json:{user:'alice',org:'team',csrf_token:'csrf',products:[{product:'alpha',services:['backend'],can_review:false,policy_revision:1}]}}));
 await page.route('**/code-attribution',r=>r.fulfill({json:{profile:null,can_review:false,repositories:[],mappings:[],contributions:[{id:'c1',owner:'alice',submitted_by:'alice',revision:2,code_revision:'a'.repeat(40),targets:[{path:'backend/retry.rs'}],confirmed_by:null,reviewed_by:null}],anchors:[],truncated:false}}));
 let profileBody:Record<string,unknown>|undefined;let confirmationHeaders:Record<string,string>|undefined;
 await page.route('**/code-profile',r=>{profileBody=r.request().postDataJSON();return r.fulfill({json:{revision:1}});});
 await page.route('**/code-contributions/c1/confirm',r=>{confirmationHeaders=r.request().headers();return r.fulfill({json:{id:'c1',revision:3}});});
 await page.goto('/');
 await page.getByRole('button',{name:'Code attribution'}).click();
 await expect(page.getByRole('heading',{name:'Your code attribution'})).toBeVisible();
 await page.getByLabel('Name teammates may see').fill('Alice');
 await page.getByRole('button',{name:'Save profile'}).click();
 expect(profileBody).toMatchObject({display_name:'Alice',services:['backend']});
 await page.getByRole('checkbox',{name:/I confirm I made this contribution/}).check();
 await page.getByRole('button',{name:'Confirm contribution'}).click();
 expect(confirmationHeaders?.['x-csrf-token']).toBe('csrf');
 expect(confirmationHeaders?.authorization).toBeUndefined();
 await expect(page.getByText('Contribution confirmed.')).toBeVisible();
});
