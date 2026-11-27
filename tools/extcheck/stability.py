"""Repeated-reference diagnostics. Raw field paths stay in private output only."""
import json
from collections import Counter
from pathlib import Path
from PIL import Image, ImageChops
from run import validate_trace, exact, family, visual_diff


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
    interactions=[json.loads((Path(p)/"interactions.json").read_text()) for p in captures]
    interaction_paths=set()
    for current in interactions[1:]:
        interaction_paths.update(differing_paths(interactions[0],current))
    first=traces[0]; paths=set(); families=Counter(); pixels=[]
    for current in traces[1:]:
        found=differing_paths(first,current)
        paths.update(found)
        for pointer in found:
            row=pointer.split('/')[1]
            name=first[int(row)].get('name','') if row.isdigit() and int(row)<len(first) else ''
            families[family(name)]+=1
    for label in labels:
        images=[]
        for p in captures:
            with Image.open(Path(p)/(label+'.png')) as source:images.append(source.convert('RGBA'))
        for index,image in enumerate(images[1:],1):
            if visual_diff(Path(captures[0])/(label+".png"),Path(captures[index])/(label+".png"))["equal"]:continue
            if image.size!=images[0].size:pixels.append({'label':label,'dimensions_equal':False});continue
            delta=ImageChops.difference(images[0],image)
            bounds=delta.convert('RGB').getbbox()
            alpha=delta.getchannel('A').getbbox()
            if bounds or alpha:pixels.append({'label':label,'bounds':bounds,'alpha_bounds':alpha})
    return {'stable':not paths and not pixels and not interaction_paths,'interaction_paths':sorted(interaction_paths),'trace_paths':sorted(paths),'trace_families':dict(families),'pixel_differences':pixels}
