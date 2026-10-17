"""Validate and stage a local native bundle beside a packaged engine."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil

def stage(source, destination, runtime):
    source=source.resolve(strict=True)
    manifest=json.loads((source/'bundle.json').read_text(encoding='utf-8'))
    if manifest.get('version')!=1 or manifest.get('embedding_model')!='embeddinggemma-2' or manifest.get('relevance_model')!='laya-multilingual':
        raise ValueError('Unsupported native model bundle')
    files=manifest['sha256']
    for required in ['embeddinggemma-2.gguf','laya/model.onnx','laya/tokenizer.json','laya/config.json',runtime]:
        if required not in files: raise ValueError('Missing platform model asset: '+required)
    for name, expected in files.items():
        relative=PurePosixPath(name)
        if relative.is_absolute() or '..' in relative.parts or ':' in name or '\\' in name:
            raise ValueError('Unsafe model asset path')
        path=source/name
        if path.is_symlink() or not path.resolve().is_relative_to(source): raise ValueError('Linked model asset')
        with path.open('rb') as stream: actual=hashlib.file_digest(stream,'sha256').hexdigest()
        if actual!=expected: raise ValueError('Model checksum mismatch: '+name)
    if destination.exists():
        existing=json.loads((destination/'bundle.json').read_text(encoding='utf-8'))
        if existing!=manifest: raise ValueError('Preserving a different staged model bundle')
        for name,expected in files.items():
            with (destination/name).open('rb') as stream:
                if hashlib.file_digest(stream,'sha256').hexdigest()!=expected: raise ValueError('Staged model checksum mismatch')
        print(json.dumps({'models':str(destination),'verified_files':len(files),'reused':True}))
        return
    destination.mkdir(parents=True)
    for name in files:
        path=destination/name;path.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(source/name,path)
    shutil.copy2(source/'bundle.json',destination/'bundle.json')
    print(json.dumps({'models':str(destination),'verified_files':len(files),'bytes':sum((destination/name).stat().st_size for name in files)}))

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source',type=Path);parser.add_argument('destination',type=Path)
    parser.add_argument('--runtime',required=True,choices=['onnxruntime.dll','libonnxruntime.dylib','libonnxruntime.so'])
    args=parser.parse_args();stage(args.source,args.destination,args.runtime)
