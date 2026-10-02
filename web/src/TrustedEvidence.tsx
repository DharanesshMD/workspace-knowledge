import {useEffect,useState} from 'react';
import {ApiError,productPath,request} from './api';
type Evidence={provider:string;reference:string;code_revision:string;kind:string;card_revision:number;stale:boolean};
export function TrustedEvidence({token,product,cardId,revision}:{token:string;product:string;cardId:string;revision:number}) {
 const [items,setItems]=useState<Evidence[]|null>(null),[error,setError]=useState('');
 useEffect(()=>{const c=new AbortController();setItems(null);setError('');request<{evidence:Evidence[]}>(token,`${productPath(product)}/cards/${encodeURIComponent(cardId)}/evidence`,undefined,undefined,c.signal).then(v=>{if(!c.signal.aborted)setItems(v.evidence);}).catch(e=>{if(!c.signal.aborted)setError(e instanceof ApiError?e.message:'Evidence is unavailable.');});return()=>c.abort();},[token,product,cardId,revision]);
 return <section aria-label="Trusted evidence"><h3>Trusted evidence</h3>{error?<p role="alert">{error}</p>:items===null?<p>Loading evidence…</p>:items.length?<ul className="evidence">{items.map((e,i)=><li key={i}><strong>{e.kind.replaceAll('_',' ')} · {e.stale?'Stale':'Current'}</strong><p>{e.provider} · Card revision {e.card_revision}</p><code>{e.code_revision}</code><p>{e.reference}</p></li>)}</ul>:<p>No trusted evidence is recorded. Author-reported references alone do not verify implementation.</p>}</section>;
}
