// Use an unchanged public engine/corpus project. No private project data.
import {shapeProjects} from '../../../../engine/corpus/src/projects/shapes';
import {mkdirSync,writeFileSync} from 'node:fs';
const project=shapeProjects()[0]!;
mkdirSync('/tmp/pm-s3-corpus',{recursive:true});
writeFileSync('/tmp/pm-s3-corpus/document.json',JSON.stringify(project.doc));
console.log('Prepared public corpus shape project (unchanged)');
