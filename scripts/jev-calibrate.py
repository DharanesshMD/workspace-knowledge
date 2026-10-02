"""Offline decision calibration. Never uploads examples or grants publication permission."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics


def lower_bound(correct, total):
    if not total:
        return 0.0
    z = 1.96
    p = correct / total
    return (p + z*z/(2*total) - z*math.sqrt(p*(1-p)/total + z*z/(4*total*total))) / (1 + z*z/total)


def evaluate(rows, threshold):
    accepted = [r for r in rows if threshold is not None and r['max_probability'] >= threshold]
    correct = sum(r['correct'] for r in accepted)
    latencies = [r['latency_ms'] for r in rows if 'latency_ms' in r]
    return {'examples':len(rows),'accepted':len(accepted),'coverage':len(accepted)/len(rows) if rows else 0,
            'accepted_accuracy':correct/len(accepted) if accepted else None,
            'accuracy_lower_95':lower_bound(correct,len(accepted)),
            'median_latency_ms':statistics.median(latencies) if latencies else None}


def calibrate(rows, target):
    calibration = [r for r in rows if r['split'] == 'calibration']
    evaluation = [r for r in rows if r['split'] == 'evaluation']
    if len(calibration) < 100 or len(evaluation) < 100:
        raise ValueError('at least 100 independently labeled examples per split required')
    threshold = None
    for candidate in [n/100 for n in range(50,101)]:
        result = evaluate(calibration,candidate)
        if result['accepted'] >= 30 and result['accuracy_lower_95'] >= target:
            threshold = candidate
            break
    return {'candidate_threshold':threshold,'metric':'max_label_probability','target_lower_accuracy':target,
            'calibration':evaluate(calibration,threshold),'held_out':evaluate(evaluation,threshold),
            'permission':'advisory_only; operator review of held-out results required'}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input',type=Path);parser.add_argument('output',type=Path)
    parser.add_argument('--target',type=float,default=.95)
    args=parser.parse_args()
    data=args.input.read_bytes()
    if len(data)>10*1024*1024 or not .5<=args.target<=1: raise ValueError('invalid input bounds')
    rows=[json.loads(line) for line in data.splitlines() if line.strip()]
    for row in rows:
        if row['split'] not in ['calibration','evaluation'] or type(row['correct']) is not bool or not 0<=row['max_probability']<=1: raise ValueError('invalid labeled row')
        if 'latency_ms' in row and (not math.isfinite(row['latency_ms']) or row['latency_ms']<0): raise ValueError('invalid latency')
    versions={(r['model'],r['rubric_version'],r['task']) for r in rows}
    if len(versions)!=1: raise ValueError('calibrate one pinned model/rubric/task at a time')
    model,rubric,task=versions.pop()
    report=calibrate(rows,args.target)
    report.update(model=model,rubric_version=rubric,task=task,dataset_sha256=hashlib.sha256(data).hexdigest())
    with args.output.open('x') as output: json.dump(report,output,indent=2)
    print('Calibration report written; no examples uploaded or included in report')

if __name__=='__main__': main()
