import {useEffect,useRef,useState} from 'react';
import type {Membership} from './Session';

const NAME='workspace-knowledge';
// The server sends a validated https origin plus /mcp/{product}. Treat it as untrusted text anyway:
// only an address with no whitespace, quotes or shell metacharacters is placed in a snippet.
const usable=(url:unknown):url is string=>typeof url==='string'&&url.length<=400&&/^https:\/\/[A-Za-z0-9.:[\]-]+\/mcp\/[A-Za-z0-9_-]+$/.test(url);
const json=(value:unknown)=>JSON.stringify(value,null,2);

async function copyText(text:string):Promise<boolean> {
 try{if(navigator.clipboard?.writeText){await navigator.clipboard.writeText(text);return true;}}catch{/* fall back to the selection copy below */}
 const previous=document.activeElement as HTMLElement|null;const area=document.createElement('textarea');
 area.value=text;area.readOnly=true;area.setAttribute('aria-hidden','true');Object.assign(area.style,{position:'fixed',top:'0',left:'0',opacity:'0'});
 document.body.append(area);area.select();
 try{return document.execCommand('copy');}catch{return false;}finally{area.remove();previous?.focus?.();}
}

function CopyButton({text,label,announce}:{text:string;label:string;announce:(message:string)=>void}) {
 const [state,setState]=useState<'idle'|'copied'|'failed'>('idle');const timer=useRef<number|undefined>(undefined);
 useEffect(()=>()=>window.clearTimeout(timer.current),[]);
 const copy=async()=>{
  const ok=await copyText(text);setState(ok?'copied':'failed');
  announce(ok?`Copied ${label}.`:`Could not copy ${label}. Select the text and copy it yourself.`);
  window.clearTimeout(timer.current);timer.current=window.setTimeout(()=>setState('idle'),3000);
 };
 return <button type="button" className="copy-button" onClick={()=>void copy()}>{state==='copied'?'Copied':state==='failed'?'Copy failed':'Copy'}<span className="sr-only"> {label}</span></button>;
}

function Snippet({id,title,product,text,announce,children}:{id:string;title:string;product:string;text:string;announce:(message:string)=>void;children:React.ReactNode}) {
 const heading=`${id}-heading`;
 return <section className="snippet" aria-labelledby={heading}><h3 id={heading}>{title}</h3>{children}<pre><code>{text}</code></pre><CopyButton text={text} label={`${title} for ${product}`} announce={announce}/></section>;
}

function ProductConnect({membership,announce}:{membership:Membership;announce:(message:string)=>void}) {
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
  </>}
 </section>;
}

export function Connect({products}:{products:Membership[]}) {
 const [message,setMessage]=useState('');const [count,setCount]=useState(0);
 const announce=(text:string)=>{setMessage(text);setCount(n=>n+1);};
 return <div className="connect-view">
  <p className="intro">Connect your AI agent to your team’s knowledge. Copy the address for a product, then add it to your agent.</p>
  <section className="panel connect-about" aria-labelledby="connect-about-title"><h2 id="connect-about-title">What the agent can do</h2><ul><li>Fetch your team’s shared context for the work you are doing.</li><li>Share a screened summary of your work with the team, only when you ask it to.</li><li>It never sees your conversation. The server receives only what the agent sends when it uses one of these tools.</li></ul></section>
  {products.map(p=><ProductConnect key={p.product} membership={p} announce={announce}/>)}
  <p className="connect-pointer">You will be asked to sign in once in your browser the first time the agent connects. For approved-folder features and sharing from a project folder, install the local bridge <code>wk-mcp</code> instead (see the setup guide).</p>
  <div role="status" aria-live="polite" className="sr-only"><span key={count}>{message}</span></div>
 </div>;
}
