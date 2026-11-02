"""Repeated-reference diagnostics. Raw field paths stay in private output only."""
import json
from collections import Counter
from pathlib import Path
from PIL import Image, ImageChops
from run import validate_trace, exact, family


def differing_paths(one, two, path=''):
    if type(one) is not type(two):return [path]
    if isinstance(one,dict):
        if list(one)!=list(two):return [path+'/$keys']
        return [p for key in one for p in differing_paths(one[key],two[key],path+'/'+str(key).replace('~','~0').replace('/','~1'))]
    if isinstance(one,list):
        if len(one)!=len(two):return [path+'/$length']
        return [p for i,(a,b) in enumerate(zip(one,two)) for p in differing_paths(a,b,path+'/'+str(i))]
    return [] if exact(one)==exact(two) else [path]


def inspect_repeats(captures, labels):
    if len(captures)<3:raise ValueError('At least three reference captures are required')
    traces=[validate_trace((Path(p)/'trace.jsonl').read_text())[1:] for p in captures]
    first=traces[0]; paths=set(); families=Counter(); pixels=[]
    for current in traces[1:]:
        found=differing_paths(first,current)
        paths.update(found)
        for pointer in found:
            row=pointer.split('/')[1]
            name=first[int(row)].get('name','') if row.isdigit() and int(row)<len(first) else ''
            families[family(name)]+=1
    for label in labels:
        images=[Image.open(Path(p)/(label+'.png')).convert('RGBA') for p in captures]
        for image in images[1:]:
            if image.size!=images[0].size:pixels.append({'label':label,'dimensions_equal':False});continue
            delta=ImageChops.difference(images[0],image)
            bounds=delta.convert('RGB').getbbox()
            alpha=delta.getchannel('A').getbbox()
            if bounds or alpha:pixels.append({'label':label,'bounds':bounds,'alpha_bounds':alpha})
    return {'stable':not paths and not pixels,'trace_paths':sorted(paths),'trace_families':dict(families),'pixel_differences':pixels}
