import {useEffect,useState,type FormEvent} from 'react';
import {ApiError,request} from './api';
import {CopyButton,day,lifetimes,shaped,usable} from './common';
import {ClientChooser,setupMessage,useAgentSetup} from './AgentSetup';
import type {PersonalTokenSettings} from './Session';

type Tok={id:string;label:string;created_ms:number;expires_ms:number;last_used_ms:number|null};
type User={user:string;services:string[];can_review:boolean;token_only:boolean;tokens:Tok[]};
type Listing={services:string[];users:User[];mcp_url:string|null};
type Issued={user:string;product:string;token:string;expires_ms:number;mcp_url:string|null};

const reason=(e:unknown,what:string)=>e instanceof ApiError
 ?e.status===409?'That name is already used by another developer. Choose a different one.'
 :e.status===403?'You are not allowed to do that. Check that the services are within your admin scope.'
 :e.status===422?'The request was not valid. A name uses lowercase letters, digits, dots, dashes or underscores, starts with a letter and has at most 32 characters.'
 :`${what} Try again.`
 :`${what} Try again.`;

export function Admin({product,settings}:{product:string;settings:PersonalTokenSettings}) {
 const base=`admin-${product}`;const choices=lifetimes(settings.max_days);
 const path=`/v1/admin/products/${encodeURIComponent(product)}/users`;
 const [listing,setListing]=useState<Listing|null>(null);const [issued,setIssued]=useState<Issued|null>(null);
 const [name,setName]=useState('');const [picked,setPicked]=useState<string[]>([]);const [review,setReview]=useState(false);
 const [days,setDays]=useState(choices.includes(30)?30:choices[choices.length-1]);
 const [busy,setBusy]=useState(false);const [error,setError]=useState('');const [confirming,setConfirming]=useState<string|null>(null);
 const [message,setMessage]=useState('');const [count,setCount]=useState(0);
 const [clientId,setClientId]=useState('claude-code');const [autoShare,setAutoShare]=useState(false);
 const {setup,choices:clientChoices,failed:setupFailed}=useAgentSetup(product,autoShare);
 const announce=(text:string)=>{setMessage(text);setCount(n=>n+1);};
 const load=(signal?:AbortSignal)=>request<Listing>('',path,undefined,undefined,signal).then(setListing);
 useEffect(()=>{
  const controller=new AbortController();
  load(controller.signal).catch(e=>{if(!controller.signal.aborted)setError(reason(e,'The developer list could not be loaded.'));});
  return()=>controller.abort();
 // eslint-disable-next-line react-hooks/exhaustive-deps
 },[product]);
 const act=async(work:()=>Promise<void>,what:string)=>{setBusy(true);setError('');try{await work();}catch(e){setError(reason(e,what));}finally{setBusy(false);}};
 const create=(event:FormEvent)=>{
  event.preventDefault();
  void act(async()=>{
   const made=await request<Issued>('',path,{user:name.trim(),services:picked,can_review:review,days});
   setIssued(made);setName('');setPicked([]);setReview(false);await load();announce(`Created ${made.user}. Their token is shown only once.`);
  },'The developer could not be created.');
 };
 const reissue=(u:User)=>void act(async()=>{
  const made=await request<Issued>('',`${path}/${encodeURIComponent(u.user)}/token`,{days:Math.min(30,settings.max_days)});
  setIssued(made);await load();announce(`New token for ${u.user}. The old one no longer works.`);
 },'The token could not be reissued.');
 const remove=(u:User)=>void act(async()=>{
  await request('',`${path}/${encodeURIComponent(u.user)}`,undefined,'DELETE');
  setConfirming(null);if(issued?.user===u.user)setIssued(null);await load();announce(`Removed ${u.user}.`);
 },'The developer could not be removed.');
 const client=setup?(setup.clients.find(c=>c.id===clientId)??setup.clients[0]):undefined;
 const text=issued&&setup&&client?setupMessage(client,setup,issued):null;
 return <div className="admin-view">
  <p className="intro">Add a developer and send them their access. They never sign in: they paste the token and address you send into their agent. Each token works as that developer on this product only.</p>
  {error&&<p role="alert" className="error">{error}</p>}
  <section className="panel" aria-labelledby={`${base}-add`}>
   <h2 id={`${base}-add`}>Add a developer</h2>
   {listing==null?<p className="muted" role="status">Loading…</p>:
   <form className="admin-form" onSubmit={create}>
    <label>Name<input value={name} onChange={e=>setName(e.target.value)} required maxLength={32} pattern="[a-z][a-z0-9._\-]{0,31}" title="Lowercase letters, digits, dot, dash or underscore; start with a letter" placeholder="for example sam" autoComplete="off" spellCheck={false}/></label>
    <fieldset><legend>Services they can use</legend>{listing.services.map(s=><label className="checkbox" key={s}><input type="checkbox" checked={picked.includes(s)} onChange={e=>setPicked(p=>e.target.checked?[...p,s]:p.filter(x=>x!==s))}/>{s}</label>)}</fieldset>
    <label className="checkbox"><input type="checkbox" checked={review} onChange={e=>setReview(e.target.checked)}/>Can review cards</label>
    <label>Token valid for<select value={days} onChange={e=>setDays(Number(e.target.value))}>{choices.map(d=><option key={d} value={d}>{d} days</option>)}</select></label>
    <button type="submit" disabled={busy||!name.trim()||picked.length===0}>Create developer and token</button>
   </form>}
  </section>
  {issued&&<section className="panel token-once" aria-label="Share with developer">
   <h2>Send this to {issued.user}</h2>
   <p role="note"><strong>This is shown only once.</strong> It contains {issued.user}’s secret token. Send it over a channel you trust; if it is lost, reissue it below.</p>
   {clientChoices&&clientChoices.clients.length>0&&<>
    <ClientChooser setup={clientChoices} selected={client?.id??clientId} onSelect={setClientId} legend={`Which agent will ${issued.user} use?`}/>
    <label className="checkbox"><input type="checkbox" checked={autoShare} onChange={e=>setAutoShare(e.target.checked)}/>Tell the agent to share finished work automatically (otherwise only when asked)</label>
   </>}
   {text?<>
    <pre className="share-message"><code>{text}</code></pre>
    <CopyButton text={text} label={`message for ${issued.user}`} announce={announce}/>
   </>:!usable(issued.mcp_url)?<p role="alert" className="error">The server did not return a usable MCP address, so the message cannot be built. Ask your administrator to check WK_MCP_RESOURCE, then reissue this token.</p>
   :setupFailed?<p role="alert" className="error">The setup instructions could not be loaded, so the message cannot be built. Reload the page; the token and address below can still be copied.</p>
   :<p className="muted" role="status">Preparing the message…</p>}
   <div className="copy-row"><label htmlFor={`${base}-token`}>Token<input id={`${base}-token`} readOnly value={shaped(issued.token)?issued.token:''} onFocus={e=>e.currentTarget.select()} spellCheck={false}/></label>{shaped(issued.token)&&<CopyButton text={issued.token} label={`token for ${issued.user}`} announce={announce}/>}</div>
   {usable(issued.mcp_url)&&<div className="copy-row"><label htmlFor={`${base}-url`}>MCP URL<input id={`${base}-url`} readOnly value={issued.mcp_url} onFocus={e=>e.currentTarget.select()} spellCheck={false}/></label><CopyButton text={issued.mcp_url} label={`MCP URL for ${issued.user}`} announce={announce}/></div>}
   <button type="button" onClick={()=>setIssued(null)}>I have sent it</button>
  </section>}
  <section className="panel" aria-labelledby={`${base}-list`}>
   <h2 id={`${base}-list`}>Developers in this product</h2>
   {listing==null?<p className="muted">Loading…</p>:listing.users.length===0?<p className="muted">No developers yet.</p>:
   <ul className="token-list member-list">{listing.users.map(u=><li key={u.user}>
    <span><strong>{u.user}</strong>{u.can_review?' · reviewer':''}<br/><span className="muted">{u.services.join(', ')} · {u.token_only?'Token only (no sign-in)':'Signs in with the identity provider'}</span>
     {u.token_only&&(u.tokens.length===0?<><br/><span className="muted">No active token</span></>:u.tokens.map(t=><span key={t.id}><br/><span className="muted">Token “{t.label}” · expires {day(t.expires_ms)} · {t.last_used_ms==null?'never used':`last used ${day(t.last_used_ms)}`}</span></span>))}</span>
    <span className="member-actions">
     {u.token_only&&<button type="button" disabled={busy} onClick={()=>reissue(u)}>Reissue token<span className="sr-only"> for {u.user}</span></button>}
     {confirming===u.user
      ?<><button type="button" className="danger" disabled={busy} onClick={()=>remove(u)}>Confirm remove<span className="sr-only"> {u.user}</span></button><button type="button" onClick={()=>setConfirming(null)}>Cancel</button></>
      :<button type="button" disabled={busy} onClick={()=>setConfirming(u.user)}>Remove<span className="sr-only"> {u.user}</span></button>}
    </span>
   </li>)}</ul>}
   {confirming&&<p className="muted" role="note">Removing {confirming} revokes their access and any token at once.</p>}
  </section>
  <div role="status" aria-live="polite" className="sr-only"><span key={count}>{message}</span></div>
 </div>;
}
