import {useEffect,useRef,useState} from 'react';

export const NAME='workspace-knowledge';
// The server sends a validated https origin plus /mcp/{product}. Treat it as untrusted text anyway:
// only an address with no whitespace, quotes or shell metacharacters is placed in a snippet.
export const usable=(url:unknown):url is string=>typeof url==='string'&&url.length<=400&&/^https:\/\/[A-Za-z0-9.:[\]-]+\/mcp\/[A-Za-z0-9_-]+$/.test(url);
export const json=(value:unknown)=>JSON.stringify(value,null,2);

async function copyText(text:string):Promise<boolean> {
 try{if(navigator.clipboard?.writeText){await navigator.clipboard.writeText(text);return true;}}catch{/* fall back to the selection copy below */}
 const previous=document.activeElement as HTMLElement|null;const area=document.createElement('textarea');
 area.value=text;area.readOnly=true;area.setAttribute('aria-hidden','true');Object.assign(area.style,{position:'fixed',top:'0',left:'0',opacity:'0'});
 document.body.append(area);area.select();
 try{return document.execCommand('copy');}catch{return false;}finally{area.remove();previous?.focus?.();}
}

export function CopyButton({text,label,announce}:{text:string;label:string;announce:(message:string)=>void}) {
 const [state,setState]=useState<'idle'|'copied'|'failed'>('idle');const timer=useRef<number|undefined>(undefined);
 useEffect(()=>()=>window.clearTimeout(timer.current),[]);
 const copy=async()=>{
  const ok=await copyText(text);setState(ok?'copied':'failed');
  announce(ok?`Copied ${label}.`:`Could not copy ${label}. Select the text and copy it yourself.`);
  window.clearTimeout(timer.current);timer.current=window.setTimeout(()=>setState('idle'),3000);
 };
 return <button type="button" className="copy-button" onClick={()=>void copy()}>{state==='copied'?'Copied':state==='failed'?'Copy failed':'Copy'}<span className="sr-only"> {label}</span></button>;
}


// A token is only ever put in a snippet when it has exactly the shape the server generates.
export const shaped=(value:unknown):value is string=>typeof value==='string'&&/^wkpt_[0-9a-f]{64}$/.test(value);
export const day=(ms:number)=>new Date(ms).toLocaleDateString(undefined,{year:'numeric',month:'short',day:'numeric'});
export const lifetimes=(max:number)=>[...new Set([...[7,30,90].filter(d=>d<=max),max])].sort((a,b)=>a-b);

