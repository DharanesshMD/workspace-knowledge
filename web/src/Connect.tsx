import {useEffect,useRef,useState} from 'react';
import {ApiError,request} from './api';
import {CopyButton,NAME,day,json,lifetimes,shaped,usable} from './common';
import {AgentSetupPanel} from './AgentSetup';
import type {Membership,PersonalTokenSettings} from './Session';

function Snippet({id,title,product,text,announce,children}:{id:string;title:string;product:string;text:string;announce:(message:string)=>void;children:React.ReactNode}) {
 const heading=`${id}-heading`;
 return <section className="snippet" aria-labelledby={heading}><h3 id={heading}>{title}</h3>{children}<pre><code>{text}</code></pre><CopyButton text={text} label={`${title} for ${product}`} announce={announce}/></section>;
}

type Tok={id:string;product:string;label:string;created_ms:number;expires_ms:number;last_used_ms:number|null};
type Made=Tok&{token:string};

function Tokens({product,url,settings,announce}:{product:string;url:string;settings:PersonalTokenSettings;announce:(message:string)=>void}) {
 const base=`connect-${product}-tokens`;const choices=lifetimes(settings.max_days);
 const [list,setList]=useState<Tok[]|null>(null);const [made,setMade]=useState<Made|null>(null);
 const [label,setLabel]=useState('');const [days,setDays]=useState(choices.includes(30)?30:choices[choices.length-1]);
 const [busy,setBusy]=useState(false);const [error,setError]=useState('');
 useEffect(()=>{
  const controller=new AbortController();
  request<{tokens:Tok[]}>('','/v1/tokens',undefined,undefined,controller.signal).then(r=>setList(r.tokens.filter(t=>t.product===product))).catch(()=>{if(!controller.signal.aborted)setError('Your tokens could not be loaded. Reload the page to try again.');});
  return()=>controller.abort();
 },[product]);
 const fail=(e:unknown,what:string)=>setError(e instanceof ApiError&&e.status===409?'You already have the maximum number of active tokens for this product. Revoke one first.':`${what} Try again.`);
 const create=async(event:React.FormEvent)=>{
  event.preventDefault();setBusy(true);setError('');
  try{const r=await request<Made>('','/v1/tokens',{product,label:label.trim(),days});
   if(!shaped(r.token))throw new Error('unexpected token');
   const {token:_secret,...entry}=r;setMade(r);setList(l=>[entry,...(l??[])]);setLabel('');announce('Token created. It is shown only once.');
  }catch(e){fail(e,'The token could not be created.');}finally{setBusy(false);}
 };
 const revoke=async(t:Tok)=>{
  setBusy(true);setError('');
  try{await request('',`/v1/tokens/${encodeURIComponent(t.id)}`,undefined,'DELETE');setList(l=>(l??[]).filter(x=>x.id!==t.id));if(made?.id===t.id)setMade(null);announce(`Revoked token ${t.label}.`);}
  catch(e){fail(e,'The token could not be revoked.');}finally{setBusy(false);}
 };
 const secret=made&&shaped(made.token)?made.token:null;
 return <section className="tokens" aria-labelledby={`${base}-title`}>
  <h3 id={`${base}-title`}>Personal access token</h3>
  <p>Connect without a browser sign-in. A token acts as you on this product’s MCP address only. Treat it like a password: it is shown once, and you can revoke it here at any time.</p>
  {error&&<p role="alert" className="error">{error}</p>}
  <form className="token-form" onSubmit={e=>void create(e)}>
   <label>Name<input value={label} onChange={e=>setLabel(e.target.value)} required maxLength={64} placeholder="Work laptop, Cursor" autoComplete="off"/></label>
   <label>Valid for<select value={days} onChange={e=>setDays(Number(e.target.value))}>{choices.map(d=><option key={d} value={d}>{d} days</option>)}</select></label>
   <button type="submit" disabled={busy||!label.trim()}>Create token</button>
  </form>
  {made&&secret&&<section className="token-once" aria-label="New token">
   <p role="note"><strong>Copy this token now.</strong> It will not be shown again. Anyone who has it can use your access until it expires or you revoke it.</p>
   <div className="copy-row"><label htmlFor={`${base}-secret`}>Token<input id={`${base}-secret`} readOnly value={secret} onFocus={e=>e.currentTarget.select()} spellCheck={false}/></label><CopyButton text={secret} label={`token for ${product}`} announce={announce}/></div>
   <Snippet id={`${base}-env`} title="Environment variable" product={product} announce={announce} text={`export WK_TOKEN=${secret}`}>
    <p>Run this in the terminal where you start your agent. The snippets below read the token from it, so the configuration files hold no secret and are safe to commit.</p>
   </Snippet>
   <Snippet id={`${base}-claude-code`} title="Claude Code command with token" product={product} announce={announce} text={`claude mcp add --transport http ${NAME} ${url} --header "Authorization: Bearer $WK_TOKEN"`}>
    <p>Run this after the export above.</p>
   </Snippet>
   <Snippet id={`${base}-cursor`} title="Cursor configuration with token" product={product} announce={announce} text={json({mcpServers:{[NAME]:{url,headers:{Authorization:'Bearer ${env:WK_TOKEN}'}}}})}>
    <p>Save as <code>.cursor/mcp.json</code>, then start Cursor from that terminal so it can read <code>WK_TOKEN</code>.</p>
   </Snippet>
   <button type="button" onClick={()=>setMade(null)}>I have saved the token</button>
  </section>}
  <h4>Your tokens for this product</h4>
  {list==null?<p className="muted" role="status">Loading…</p>:list.length===0?<p className="muted">No active tokens.</p>
  :<ul className="token-list">{list.map(t=><li key={t.id}><span><strong>{t.label}</strong><br/><span className="muted">Created {day(t.created_ms)} · Expires {day(t.expires_ms)} · {t.last_used_ms==null?'Never used':`Last used ${day(t.last_used_ms)}`}</span></span><button type="button" disabled={busy} onClick={()=>void revoke(t)}>Revoke<span className="sr-only"> token {t.label}</span></button></li>)}</ul>}
  <AgentSetupPanel product={product} announce={announce}/>
 </section>;
}

function ProductConnect({membership,announce,personalTokens}:{membership:Membership;announce:(message:string)=>void;personalTokens:PersonalTokenSettings|null}) {
 const {product,mcp_url}=membership;const url=usable(mcp_url)?mcp_url:null;const base=`connect-${product}`;
 return <section className="panel connect-product" aria-labelledby={`${base}-title`}>
  <h2 id={`${base}-title`}>Product <code>{product}</code></h2>
  {mcp_url==null?<p className="empty">Your administrator has not configured the MCP address yet (WK_MCP_RESOURCE).</p>
  :!url?<p className="empty" role="note">The MCP address reported for this product cannot be used. Ask your administrator to check WK_MCP_RESOURCE.</p>
  :<>
   <div className="copy-row"><label htmlFor={`${base}-url`}>MCP server URL<input id={`${base}-url`} readOnly value={url} onFocus={e=>e.currentTarget.select()} spellCheck={false}/></label><CopyButton text={url} label={`MCP URL for ${product}`} announce={announce}/></div>
   <Snippet id={`${base}-claude`} title="Claude connector" product={product} announce={announce} text={`Name: Workspace Knowledge\nMCP server URL: ${url}`}>
    <p>In Claude on the desktop or the web, open Settings, Connectors, Add custom connector. Enter the name and the MCP server URL below.</p>
    <p className="muted">Claude’s own connectors are connected from Anthropic’s side, so the server must be reachable from the internet. This has not been tested here.</p>
   </Snippet>
   <Snippet id={`${base}-claude-code`} title="Claude Code command" product={product} announce={announce} text={`claude mcp add --transport http ${NAME} ${url}`}>
    <p>Run this in a terminal.</p>
   </Snippet>
   <Snippet id={`${base}-cursor`} title="Cursor configuration" product={product} announce={announce} text={json({mcpServers:{[NAME]:{url}}})}>
    <p>Save as <code>.cursor/mcp.json</code>.</p>
   </Snippet>
   <Snippet id={`${base}-vscode`} title="VS Code configuration" product={product} announce={announce} text={json({servers:{[NAME]:{type:'http',url}}})}>
    <p>Save as <code>.vscode/mcp.json</code>.</p>
    <p className="muted">VS Code needs your team’s access-token lifespan to be above 5 minutes. This is set by your administrator in the identity provider.</p>
   </Snippet>
   {personalTokens&&<Tokens product={product} url={url} settings={personalTokens} announce={announce}/>}
  </>}
 </section>;
}

export function Connect({products,personalTokens}:{products:Membership[];personalTokens:PersonalTokenSettings|null}) {
 const [message,setMessage]=useState('');const [count,setCount]=useState(0);
 const announce=(text:string)=>{setMessage(text);setCount(n=>n+1);};
 return <div className="connect-view">
  <p className="intro">Connect your AI agent to your team’s knowledge. Copy the address for a product, then add it to your agent.</p>
  <section className="panel connect-about" aria-labelledby="connect-about-title"><h2 id="connect-about-title">What the agent can do</h2><ul><li>Fetch your team’s shared context for the work you are doing.</li><li>Share a screened summary of your work with the team, only when you ask it to.</li><li>It never sees your conversation. The server receives only what the agent sends when it uses one of these tools.</li></ul></section>
  {products.map(p=><ProductConnect key={p.product} membership={p} announce={announce} personalTokens={personalTokens}/>)}
  <p className="connect-pointer">You will be asked to sign in once in your browser the first time the agent connects. For approved-folder features and sharing from a project folder, install the local bridge <code>wk-mcp</code> instead (see the setup guide).</p>
  <div role="status" aria-live="polite" className="sr-only"><span key={count}>{message}</span></div>
 </div>;
}
