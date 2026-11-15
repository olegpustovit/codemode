// Fragment shader from the approved control-plane proposal.
export const fragmentShader = `precision highp float;uniform vec2 r;uniform float t;uniform vec3 h;
  /* XMB-style ribbon drawn as a bundle of strands: centre wave c(x), signed width w(x) that pinches and twists */
  float cen(float x,float k){return .30+k*.03+.105*sin(x*1.10+t*.20+k)+.040*sin(x*2.45-t*.16+1.2+k*2.)+.010*sin(x*5.2+t*.31);}
  float wid(float x,float k){return .17*sin(x*.80-t*.12+.7+k*1.7)+.045*sin(x*1.9+t*.18+k);}
  /* strand coordinate; the warp depends on v itself, so strands curl apart and together like a flow field */
  float g(vec2 p,float k,float n){float v=(p.y-cen(p.x,k))/wid(p.x,k);
    v+=.11*sin(p.x*2.3+v*4.2+t*.33+k)+.06*sin(p.x*5.1-v*7.0-t*.26)+.03*sin(p.x*9.0+v*11.-t*.41);
    return v*n;}
  /* sig: strand blur in pixels (depth of field) */
  float strands(vec2 p,float px,float k,float n,float sig,out float a){
    float v=g(p,k,n);
    a=abs(v)/(n*.5);
    if(a>1.15) return 0.;
    vec2 gr=vec2(g(p+vec2(px,0.),k,n)-v,g(p+vec2(0.,px),k,n)-v);
    float dw=max(length(gr),1e-4);              /* strands per pixel */
    float d=abs(fract(v+.5)-.5)/dw;             /* pixels to nearest strand */
    float s=exp(-d*d/(2.*sig*sig))*pow(1.1/sig,.6); /* gaussian; blurrier strands are dimmer, as out-of-focus light is */
    float avg=min(dw*sig*2.5,1.)*.5;            /* what a dense bundle averages to */
    return mix(s,avg,smoothstep(.10,.22,dw*sig));/* strands closer than ~5 sigma blend into a glow before moire can form */
  }
  vec3 ribbon(vec2 p,float px,float k,float n,float amp,float sig){
    float a0,a1;
    float l=.5*(strands(p,px,k,n,sig,a0)+strands(p+vec2(.5*px,.5*px),px,k,n,sig,a1));
    float body=(1.-smoothstep(.7,1.,a0))*(.5+.5*(1.-a0));
    float pulse=.75+.5*pow(.5+.5*sin(p.x*1.6-t*.55+k),4.);   /* light travelling along the ribbon */
    return mix(vec3(.80,.86,1.),h*1.5+.12,.35)*l*body*pulse*amp;}
  void main(){
    vec2 uv=gl_FragCoord.xy/r; float a=r.x/r.y; vec2 p=vec2(uv.x*a,uv.y); float px=1./r.y;
    vec3 col=mix(vec3(.006,.008,.018),h*.16,smoothstep(.0,1.,uv.y));
    col+=h*.10*exp(-5.*length(vec2((uv.x-.78)*1.2,uv.y-1.)));
    float c=cen(p.x,0.); col+=mix(h,vec3(1.),.3)*.11*exp(-pow((uv.y-c)*6.,2.));   /* bloom under the ribbon */
    /* sharp only near a slowly wandering focal point, otherwise soft; keeps text over it readable */
    float fx=a*(.15+.12*sin(t*.07)); float focus=exp(-pow((p.x-fx)*2.2,2.));
    col+=ribbon(p,px,0.,15.,.55,mix(3.2,.9,focus)/px*px);
    col+=ribbon(p,px,2.3,8.,.30,5.);
    col*=1.-.28*length(vec2(uv.x-.55,uv.y-.5));
    gl_FragColor=vec4(col,1.);}`
