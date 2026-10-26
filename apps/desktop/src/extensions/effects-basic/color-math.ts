/** Independent D65 matrix derivation from standardized RGB chromaticities.
 * Explicit effect transforms only; this does not assign a project working space. */
export type RGB=readonly[number,number,number];
export type Matrix=readonly[number,number,number,number,number,number,number,number,number];
export const COLOR_PROFILES=['sRGB','Display P3','Rec.2020 (display gamma 2.4)'];
export function multiply(m:Matrix,v:RGB):[number,number,number]{return[m[0]*v[0]+m[1]*v[1]+m[2]*v[2],m[3]*v[0]+m[4]*v[1]+m[5]*v[2],m[6]*v[0]+m[7]*v[1]+m[8]*v[2]];}
export function inverse(m:Matrix):Matrix{const[a,b,c,d,e,f,g,h,i]=m,A=e*i-f*h,B=c*h-b*i,C=b*f-c*e,D=f*g-d*i,E=a*i-c*g,F=c*d-a*f,G=d*h-e*g,H=b*g-a*h,I=a*e-b*d,det=a*A+b*D+c*G;if(Math.abs(det)<1e-12)throw new Error('Singular color matrix');return[A/det,B/det,C/det,D/det,E/det,F/det,G/det,H/det,I/det];}
function primaryMatrix(p:Array<readonly[number,number]>):Matrix{const columns=p.map(([x,y])=>[x/y,1,(1-x-y)/y]),m=[...columns[0]!,...columns[1]!,...columns[2]!] as unknown as Matrix,rows=[m[0],m[3],m[6],m[1],m[4],m[7],m[2],m[5],m[8]] as Matrix,white:[number,number,number]=[.3127/.329,1,(1-.3127-.329)/.329],scale=multiply(inverse(rows),white);return rows.map((v,i)=>v*scale[i%3]!) as unknown as Matrix;}
export const RGB_TO_XYZ:Matrix[]=[
 primaryMatrix([[.64,.33],[.3,.6],[.15,.06]]),
 primaryMatrix([[.68,.32],[.265,.69],[.15,.06]]),
 primaryMatrix([[.708,.292],[.17,.797],[.131,.046]]),
];
export const XYZ_TO_RGB=RGB_TO_XYZ.map(inverse);
/** Odd extension retains signed HDR values; Rec.2020 here is the display EOTF. */
export function decodeTransfer(x:number,profile:number):number{const a=Math.abs(x);return Math.sign(x)*(profile===2?a**2.4:a<=.04045?a/12.92:((a+.055)/1.055)**2.4);}
export function encodeTransfer(x:number,profile:number):number{const a=Math.abs(x);return Math.sign(x)*(profile===2?a**(1/2.4):a<=.0031308?12.92*a:1.055*a**(1/2.4)-.055);}
export function convertProfile(rgb:RGB,input:number,output:number,linearInput=false,linearOutput=false):[number,number,number]{if(!Number.isInteger(input)||!Number.isInteger(output)||input<0||input>2||output<0||output>2)throw new Error('Unknown color profile');if(input===output&&linearInput===linearOutput)return[...rgb];const linear=rgb.map(x=>linearInput?x:decodeTransfer(x,input)) as [number,number,number],out=input===output?linear:multiply(XYZ_TO_RGB[output]!,multiply(RGB_TO_XYZ[input]!,linear));return out.map(x=>linearOutput?x:encodeTransfer(x,output)) as [number,number,number];}
export const RGB_TO_YIQ:Matrix=[.299,.587,.114,.595716,-.274453,-.321263,.211456,-.522591,.311135];
export const RGB_TO_YUV:Matrix=[.299,.587,.114,-.14713,-.28886,.436,.615,-.51499,-.10001];
export const RGB_TO_YCBCR:Matrix=[.2126,.7152,.0722,-.2126/1.8556,-.7152/1.8556,.5,.5,-.7152/1.5748,-.0722/1.5748];
export function matrixGLSL(m:Matrix){return'mat3('+[0,3,6,1,4,7,2,5,8].map(i=>{const v=Number(m[i]!.toPrecision(15));return Number.isInteger(v)?v+'.':String(v);}).join(',')+')';}
export const COLOR_SPACE_GLSL=`
float decodeTransfer(float x,int profile){float a=abs(x);return sign(x)*(profile==2?pow(a,2.4):a<=.04045?a/12.92:pow((a+.055)/1.055,2.4));}
float encodeTransfer(float x,int profile){float a=abs(x);return sign(x)*(profile==2?pow(a,1./2.4):a<=.0031308?12.92*a:1.055*pow(a,1./2.4)-.055);}
mat3 toXYZ(int profile){${RGB_TO_XYZ.map((m,i)=>`${i<2?`if(profile==${i})`:''}return ${matrixGLSL(m)};`).join('')}}
mat3 fromXYZ(int profile){${XYZ_TO_RGB.map((m,i)=>`${i<2?`if(profile==${i})`:''}return ${matrixGLSL(m)};`).join('')}}
vec3 convertProfile(vec3 rgb,int inputProfile,int outputProfile,bool linearInput,bool linearOutput){
 if(inputProfile==outputProfile&&linearInput==linearOutput)return rgb;
 vec3 linear=linearInput?rgb:vec3(decodeTransfer(rgb.r,inputProfile),decodeTransfer(rgb.g,inputProfile),decodeTransfer(rgb.b,inputProfile));
 vec3 result=inputProfile==outputProfile?linear:fromXYZ(outputProfile)*toXYZ(inputProfile)*linear;
 return linearOutput?result:vec3(encodeTransfer(result.r,outputProfile),encodeTransfer(result.g,outputProfile),encodeTransfer(result.b,outputProfile));
}
`;
