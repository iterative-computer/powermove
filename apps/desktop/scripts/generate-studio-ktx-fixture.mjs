import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { deflateSync } from 'node:zlib';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const directory=mkdtempSync(path.join(tmpdir(),'pm-ktx-fixture-'));
const crc=bytes=>{let crc=0xffffffff;for(const byte of bytes){crc^=byte;for(let i=0;i<8;i++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}return (crc^0xffffffff)>>>0;};
const chunk=(type,data)=>{const name=Buffer.from(type),size=Buffer.alloc(4),sum=Buffer.alloc(4);size.writeUInt32BE(data.length);sum.writeUInt32BE(crc(Buffer.concat([name,data])));return Buffer.concat([size,name,data,sum]);};
try {
  const width=32,height=32,pixels=Buffer.alloc((width*4+1)*height),colors=[[255,0,0,255],[0,255,0,255],[0,0,255,255],[255,255,0,255]];
  for(let y=0;y<height;y++)for(let x=0;x<width;x++)pixels.set(colors[(y>=16?2:0)+(x>=16?1:0)],y*(width*4+1)+1+x*4);
  const header=Buffer.alloc(13);header.writeUInt32BE(width,0);header.writeUInt32BE(height,4);header[8]=8;header[9]=6;
  const png=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(pixels)),chunk('IEND',Buffer.alloc(0))]);
  const source=path.join(directory,'quadrants.png'),output=fileURLToPath(new URL('../e2e/fixtures/tiny-basis.ktx2',import.meta.url));writeFileSync(source,png);
  execFileSync('basisu',['-ktx2','-uastc','-uastc_level','0','-no_multithreading','-file',source,'-output_file',output],{stdio:'inherit'});
} finally {rmSync(directory,{recursive:true,force:true});}
