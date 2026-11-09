import {useEffect,useState} from 'react';
import {request} from './api';
import {CopyButton,day,usable} from './common';

export type SetupFile={path:string;kind:'config'|'rules';format:string;content:string;merge:string};
export type ClientSetup={id:string;name:string;summary:string;steps:string[];files:SetupFile[];command:string|null;verify:string;notes:string[];docs:string};
export type AgentSetup={product:string;mcp_url:string|null;token_env:string;token_placeholder:string;rules:string;clients:ClientSetup[]};
export type Handover={user:string;product:string;token:string;expires_ms:number;mcp_url:string|null};

// The server never sends a token. Where a setup needs the real value (an export line, or Claude Desktop's own
// config), the admin's message fills it in; it only ever does so for a token with the exact shape the server makes.
const TOKEN_SHAPE=/^wkpt_[0-9a-f]{64}$/;
const fill=(text:string,token:string|null,placeholder:string)=>token&&TOKEN_SHAPE.test(token)?text.replaceAll('<your token>',token).replaceAll(placeholder,token):text;
const docsLink=(url:string)=>/^https:\/\/[A-Za-z0-9./?=&_#:-]+$/.test(url)?url:null;

/** One message an admin can send a developer: address, token, and exactly what to do for the chosen agent. */
export function setupMessage(client:ClientSetup,setup:AgentSetup,who:Handover):string|null {
 if(!TOKEN_SHAPE.test(who.token)||!usable(who.mcp_url))return null;
 const f=(t:string)=>fill(t,who.token,setup.token_placeholder);
 const lines=[
  `Workspace Knowledge access for ${who.user} (product ${who.product})`,'',
  `MCP URL: ${who.mcp_url}`,
  `Token (keep it private, valid until ${day(who.expires_ms)}): ${who.token}`,'',
  `Setup for ${client.name}`,
  ...client.steps.map((s,i)=>`${i+1}. ${f(s)}`),
 ];
 for(const file of client.files){
  lines.push('',`${file.kind==='rules'?'Standing instructions':'Connection'}: save as ${file.path}`,`(${file.merge})`,f(file.content));
 }
 if(client.command)lines.push('',`Or in one command: ${f(client.command)}`);
 for(const n of client.notes)lines.push('',`Note: ${f(n)}`);
 lines.push('',`Check it works: ${client.verify}`,'','If you lose the token, ask me for a new one; the old one then stops working.');
 return lines.join('\n');
}

export function useAgentSetup(product:string,autoShare:boolean) {
 const [setup,setSetup]=useState<AgentSetup|null>(null);const [failed,setFailed]=useState(false);
 useEffect(()=>{
  const controller=new AbortController();setFailed(false);
  request<AgentSetup>('',`/v1/products/${encodeURIComponent(product)}/agent-setup${autoShare?'?auto_share=true':''}`,undefined,undefined,controller.signal)
   .then(s=>{if(!Array.isArray(s?.clients))throw new Error('unexpected');setSetup(s);})
   .catch(()=>{if(!controller.signal.aborted)setFailed(true);});
  return()=>controller.abort();
 },[product,autoShare]);
 return {setup,failed};
}

/** The five clients as a radio group, with the chosen one's files, command and notes. */
export function ClientChooser({setup,selected,onSelect,legend}:{setup:AgentSetup;selected:string;onSelect:(id:string)=>void;legend:string}) {
 return <fieldset className="client-chooser"><legend>{legend}</legend>{setup.clients.map(c=><label key={c.id} className="client-option"><input type="radio" name={`client-${legend}`} value={c.id} checked={selected===c.id} onChange={()=>onSelect(c.id)}/>{c.name}</label>)}</fieldset>;
}

export function AgentSetupPanel({product,announce}:{product:string;announce:(message:string)=>void}) {
 const [autoShare,setAutoShare]=useState(false);const {setup,failed}=useAgentSetup(product,autoShare);
 const [selected,setSelected]=useState('claude-code');
 const base=`setup-${product}`;
 if(failed)return <section className="agent-setup" aria-labelledby={`${base}-title`}><h3 id={`${base}-title`}>Set up your coding agent</h3><p className="muted">The setup instructions could not be loaded. Reload the page to try again.</p></section>;
 if(!setup||setup.clients.length===0||!usable(setup.mcp_url))return null;
 const client=setup.clients.find(c=>c.id===selected)??setup.clients[0];
 const link=docsLink(client.docs);
 return <section className="agent-setup" aria-labelledby={`${base}-title`}>
  <h3 id={`${base}-title`}>Set up your coding agent</h3>
  <p>Pick your agent. These files connect it using your token and give it standing instructions, so it checks what teammates did before editing and can answer questions about their work. Nothing below contains your token.</p>
  <ClientChooser setup={setup} selected={client.id} onSelect={setSelected} legend="Your coding agent"/>
  <label className="checkbox"><input type="checkbox" checked={autoShare} onChange={e=>setAutoShare(e.target.checked)}/>Tell the agent to share finished work automatically (otherwise it shares only when you ask)</label>
  <p>{client.summary}</p>
  <ol>{client.steps.map((s,i)=><li key={i}>{s}</li>)}</ol>
  {client.files.map(f=><div className="snippet" key={f.path}>
   <h4>{f.kind==='rules'?'Standing instructions':'Connection'}: save as <code>{f.path}</code></h4>
   <p className="muted">{f.merge}</p>
   <pre><code>{f.content}</code></pre>
   <CopyButton text={f.content} label={`${f.path} for ${client.name}`} announce={announce}/>
  </div>)}
  {client.command&&<div className="snippet"><h4>Or in one command</h4><pre><code>{client.command}</code></pre><CopyButton text={client.command} label={`${client.name} command`} announce={announce}/></div>}
  {client.notes.length>0&&<ul className="muted">{client.notes.map((n,i)=><li key={i}>{n}</li>)}</ul>}
  <p><strong>Check it works:</strong> {client.verify}</p>
  {link&&<p><a href={link} target="_blank" rel="noopener noreferrer">{client.name} documentation</a></p>}
 </section>;
}
