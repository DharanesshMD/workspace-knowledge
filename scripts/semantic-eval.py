#!/usr/bin/env python3
"""Compare two explicitly selected local APIs using synthetic reviewed records."""
import argparse
import json
import os
from pathlib import Path
import statistics
import time
import urllib.parse
import urllib.request

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--keyword-url',required=True)
    parser.add_argument('--hybrid-url',required=True)
    parser.add_argument('--corpus',type=Path,default=Path(__file__).parent/'fixtures/semantic-search.json')
    parser.add_argument('--ids',type=Path,required=True)
    parser.add_argument('--output',type=Path)
    parser.add_argument('--seed',action='store_true')
    parser.add_argument('--seed-only',action='store_true')
    args=parser.parse_args()
    if os.environ.get('WK_SEMANTIC_EVAL_SYNTHETIC')!='1':
        parser.error('Set WK_SEMANTIC_EVAL_SYNTHETIC=1 for this explicitly synthetic fixture')
    for base in [args.keyword_url,args.hybrid_url]:
        url=urllib.parse.urlsplit(base)
        if url.scheme!='http' or url.hostname not in ['127.0.0.1','::1'] or url.path not in ['', '/'] or url.username or url.password or url.query or url.fragment:
            parser.error('Only explicitly selected literal-loopback HTTP API origins are supported')
    corpus=json.loads(args.corpus.read_text())
    credential=os.environ['WK_ACCESS_TOKEN']
    opener=urllib.request.build_opener(NoRedirect())
    def post(base,path,body):
        request=urllib.request.Request(base.rstrip('/')+path,data=json.dumps(body).encode(),headers={'Content-Type':'application/json','Authorization':'Bearer '+credential})
        with opener.open(request,timeout=10) as response:
            raw=response.read(2*1024*1024+1)
        if len(raw)>2*1024*1024:raise ValueError('bounded evaluation response exceeded')
        return json.loads(raw)
    if args.seed:
        ids={}
        for record in corpus['records']:
            card={'schema_version':1,'idempotency_key':corpus['name']+'-'+record['id'],'policy_revision':1,'title':record['title'],'intent':record['intent'],'rationale':record['rationale'],'decisions':[],'constraints':[],'open_questions':[],'service_ids':['backend'],'evidence':[],'lifecycle':'proposed','capture_completeness':'complete','base_revision':None}
            saved=post(args.keyword_url,'/v1/cards',{'product':'demo','card':card})['card']
            ids[record['id']]=saved['id']
            post(args.keyword_url,'/v1/cards/'+saved['id']+'/publish',{'product':'demo'})
        args.ids.write_text(json.dumps(ids,indent=2))
    if args.seed_only:
        print(json.dumps({'synthetic_records_seeded':len(corpus['records'])}));return
    ids=json.loads(args.ids.read_text())
    results=[]
    for record in corpus['records']:
        for kind in ['query','exact']:
            if kind not in record:continue
            row={'record':record['id'],'kind':'paraphrase' if kind=='query' else 'exact'}
            for mode,base in [('keyword',args.keyword_url),('hybrid',args.hybrid_url)]:
                start=time.monotonic();response=post(base,'/v1/search',{'product':'demo','query':record[kind],'limit':5});elapsed=(time.monotonic()-start)*1000
                returned=[card['id'] for card in response['cards']]
                rank=returned.index(ids[record['id']])+1 if ids[record['id']] in returned else None
                row[mode]={'rank':rank,'latency_ms':round(elapsed,2),'retrieval':response.get('retrieval')}
            results.append(row)
    for query in corpus.get('unanswerable',[]):
        row={'record':query['id'],'kind':'unanswerable'}
        for mode,base in [('keyword',args.keyword_url),('hybrid',args.hybrid_url)]:
            start=time.monotonic();response=post(base,'/v1/search',{'product':'demo','query':query['query'],'limit':5})
            row[mode]={'returned_count':len(response['cards']),'latency_ms':round((time.monotonic()-start)*1000,2),'retrieval':response.get('retrieval')}
        results.append(row)
    summary={}
    for kind in ['paraphrase','exact']:
        subset=[r for r in results if r['kind']==kind]
        summary[kind]={}
        for mode in ['keyword','hybrid']:
            latencies=sorted(r[mode]['latency_ms'] for r in subset)
            summary[kind][mode]={'queries':len(subset),'hit_at_1':sum(r[mode]['rank']==1 for r in subset)/len(subset),'recall_at_5':sum(r[mode]['rank'] is not None for r in subset)/len(subset),'mrr_at_5':sum(1/r[mode]['rank'] if r[mode]['rank'] else 0 for r in subset)/len(subset),'latency_p50_ms':statistics.median(latencies),'latency_p95_ms':latencies[min(len(latencies)-1,int(len(latencies)*.95))]}
    unanswerable=[r for r in results if r['kind']=='unanswerable']
    if unanswerable:summary['unanswerable']={mode:{'queries':len(unanswerable),'empty_results':sum(r[mode]['returned_count']==0 for r in unanswerable)} for mode in ['keyword','hybrid']}
    report={'corpus':corpus['name'],'synthetic':True,'limits':'One local model, one sequential request stream, synthetic records; not production/team quality or capacity certification','summary':summary,'results':results}
    if args.output:args.output.write_text(json.dumps(report,indent=2))
    print(json.dumps(summary,indent=2))

if __name__=='__main__':
    main()
