#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform float u_channel, u_count;
uniform float u_x0, u_x1, u_x2, u_x3, u_x4, u_x5, u_x6, u_x7, u_x8, u_x9, u_x10, u_x11, u_x12, u_x13, u_x14;
uniform float u_y0, u_y1, u_y2, u_y3, u_y4, u_y5, u_y6, u_y7, u_y8, u_y9, u_y10, u_y11, u_y12, u_y13, u_y14;
out vec4 o;
// One curve through `n` points with distinct In (at least 1e-6 apart): a
// monotone cubic (Fritsch-Carlson tangents, the end tangents the end
// secants), straight beyond the first and last point.
float pm_slope(float xs[15],float ys[15],int i){return (ys[i+1]-ys[i])/(xs[i+1]-xs[i]);}
float pm_tangent(float xs[15],float ys[15],int n,int i){
 if(i<=0)return pm_slope(xs,ys,0);
 if(i>=n-1)return pm_slope(xs,ys,n-2);
 float a=pm_slope(xs,ys,i-1),b=pm_slope(xs,ys,i);
 if(a*b<=0.)return 0.;
 float ha=xs[i]-xs[i-1],hb=xs[i+1]-xs[i];
 return 3.*(ha+hb)/((2.*hb+ha)/a+(hb+2.*ha)/b);
}
float pm_curve(float xs[15],float ys[15],int n,float v){
 // One point left: the line of slope 1 through it.
 if(n<2)return ys[0]+(v-xs[0]);
 if(v<=xs[0])return ys[0]+pm_tangent(xs,ys,n,0)*(v-xs[0]);
 if(v>=xs[n-1])return ys[n-1]+pm_tangent(xs,ys,n,n-1)*(v-xs[n-1]);
 int i=0;
 for(int k=1;k<15-1;k++)if(k<n-1&&v>=xs[k])i=k;
 float h=xs[i+1]-xs[i],t=(v-xs[i])/h,t2=t*t,t3=t2*t;
 return (2.*t3-3.*t2+1.)*ys[i]+(t3-2.*t2+t)*h*pm_tangent(xs,ys,n,i)+(-2.*t3+3.*t2)*ys[i+1]+(t3-t2)*h*pm_tangent(xs,ys,n,i+1);
}
void main(){
 vec4 c=texture(u_tex,v_st);if(c.a<=0.){o=c;return;}
 int n=int(clamp(u_count,2.,15.)+.5);
 float xs[15]=float[15](u_x0,u_x1,u_x2,u_x3,u_x4,u_x5,u_x6,u_x7,u_x8,u_x9,u_x10,u_x11,u_x12,u_x13,u_x14),ys[15]=float[15](u_y0,u_y1,u_y2,u_y3,u_y4,u_y5,u_y6,u_y7,u_y8,u_y9,u_y10,u_y11,u_y12,u_y13,u_y14);
 // Points are read in input order, however they were written.
 int id[15]=int[15](0,1,2,3,4,5,6,7,8,9,10,11,12,13,14);
 for(int i=1;i<15;i++){if(i>=n)break;float kx=xs[i],ky=ys[i];int ki=id[i];int j=i-1;while(j>=0&&xs[j]>kx){xs[j+1]=xs[j];ys[j+1]=ys[j];id[j+1]=id[j];j--;}xs[j+1]=kx;ys[j+1]=ky;id[j+1]=ki;}
 // Points whose In lie within 1e-6 of the one before are one point: the one
 // written last survives (an added point waiting on white changes nothing).
 int m=1;float prev=xs[0];
 for(int i=1;i<15;i++){if(i>=n)break;float kx=xs[i];
  if(kx-prev<1e-6){if(id[i]>id[m-1]){xs[m-1]=kx;ys[m-1]=ys[i];id[m-1]=id[i];}}
  else{xs[m]=kx;ys[m]=ys[i];id[m]=id[i];m++;}
  prev=kx;}
 n=m;
 if(n==2&&xs[0]==0.&&ys[0]==0.&&xs[1]==1.&&ys[1]==1.){o=c;return;}
 vec3 x=c.rgb/c.a,y=x;
 int channel=int(u_channel+.5);
 if(channel==0||channel==1)y.r=pm_curve(xs,ys,n,x.r);
 if(channel==0||channel==2)y.g=pm_curve(xs,ys,n,x.g);
 if(channel==0||channel==3)y.b=pm_curve(xs,ys,n,x.b);
 // A channel the curve leaves alone keeps its stored bits.
 vec3 graded=y*c.a;
 o=vec4(y.r==x.r?c.r:graded.r,y.g==x.g?c.g:graded.g,y.b==x.b?c.b:graded.b,c.a);
}
