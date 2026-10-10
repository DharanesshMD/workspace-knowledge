import {test,expect} from '@playwright/test';
test('hosted anonymous portal offers organization sign-in without token input',async({page})=>{
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({status:401,json:{error:'unauthenticated'}}));
 await page.goto('/');await expect(page.getByRole('link',{name:'Sign in with your organization'})).toBeVisible();await expect(page.getByLabel('Access token')).toHaveCount(0);
});
test('permitted products switch context and logout uses session CSRF',async({page})=>{
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({json:{user:'alice',org:'team',csrf_token:'csrf-test',products:[{product:'alpha',services:['backend'],can_review:false,policy_revision:1},{product:'beta',services:['frontend'],can_review:true,policy_revision:1}]}}));
 await page.route('**/v1/search',r=>r.fulfill({json:{cards:[]}}));
 let loggedOut=false;await page.route('**/auth/logout',r=>{expect(r.request().method()).toBe('POST');expect(r.request().headers()['x-csrf-token']).toBe('csrf-test');loggedOut=true;return r.fulfill({json:{logged_out:true}});});
 await page.goto('/');await expect(page.getByLabel('Current product')).toHaveValue('alpha');await page.getByLabel('Search product knowledge').fill('something');await page.getByRole('button',{name:'Search',exact:true}).click();await expect(page.getByText('No matching knowledge is visible')).toBeVisible();await page.getByLabel('Current product').selectOption('beta');await expect(page.getByLabel('Search product knowledge')).toHaveValue('');await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('link',{name:'Sign in with your organization'})).toBeVisible();expect(loggedOut).toBe(true);
});
test('desktop session controls fit the initial viewport',async({page})=>{
 await page.setViewportSize({width:1040,height:853});
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));
 await page.route('**/v1/me',r=>r.fulfill({json:{user:'alice',org:'team',csrf_token:'csrf',products:[{product:'beta',services:['backend'],can_review:false,policy_revision:1}]}}));
 await page.goto('/');await expect(page.getByRole('button',{name:'Sign out',exact:true})).toBeInViewport({ratio:1});
 await page.screenshot({path:'test-results/hosted-desktop.png',fullPage:true});
 await page.setViewportSize({width:390,height:844});expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
 await page.screenshot({path:'test-results/hosted-mobile.png',fullPage:true});
});
test('zero membership and narrow screens reveal no invented product',async({page})=>{
 await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:true,development:false}}));await page.route('**/v1/me',r=>r.fulfill({json:{user:'alice',org:'team',csrf_token:'csrf',products:[]}}));await page.setViewportSize({width:390,height:844});await page.goto('/');await expect(page.getByText('No products are assigned to your account.')).toBeVisible();await expect(page.getByLabel('Current product')).toHaveCount(0);expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
});
test('unavailable production configuration does not expose token fallback',async({page})=>{
 await page.route('**/auth/config',r=>r.fulfill({status:503,json:{error:'unavailable'}}));await page.goto('/');await expect(page.getByRole('alert')).toBeVisible();await expect(page.getByLabel('Access token')).toHaveCount(0);
});
