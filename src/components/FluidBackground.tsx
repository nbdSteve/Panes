import { useRef, useEffect } from "react";

const VERTEX = `attribute vec2 position;
void main() { gl_Position = vec4(position, 0.0, 1.0); }`;

const FRAGMENT = `precision highp float;
uniform float iTime;
uniform vec2 iResolution;

vec3 mod289(vec3 x){return x-floor(x*(1./289.))*289.;}
vec2 mod289(vec2 x){return x-floor(x*(1./289.))*289.;}
vec3 permute(vec3 x){return mod289(((x*34.)+1.)*x);}

float snoise(vec2 v){
  const vec4 C=vec4(.211324865405187,.366025403784439,-.577350269189626,.024390243902439);
  vec2 i=floor(v+dot(v,C.yy));
  vec2 x0=v-i+dot(i,C.xx);
  vec2 i1=(x0.x>x0.y)?vec2(1.,0.):vec2(0.,1.);
  vec4 x12=x0.xyxy+C.xxzz;
  x12.xy-=i1;
  i=mod289(i);
  vec3 p=permute(permute(i.y+vec3(0.,i1.y,1.))+i.x+vec3(0.,i1.x,1.));
  vec3 m=max(.5-vec3(dot(x0,x0),dot(x12.xy,x12.xy),dot(x12.zw,x12.zw)),0.);
  m=m*m;m=m*m;
  vec3 x=2.*fract(p*C.www)-1.;
  vec3 h=abs(x)-.5;
  vec3 ox=floor(x+.5);
  vec3 a0=x-ox;
  m*=1.79284291400159-.85373472095314*(a0*a0+h*h);
  vec3 g;
  g.x=a0.x*x0.x+h.x*x0.y;
  g.yz=a0.yz*x12.xz+h.yz*x12.yw;
  return 130.*dot(m,g);
}

void main(){
  vec2 uv=(gl_FragCoord.xy*2.-iResolution.xy)/min(iResolution.x,iResolution.y);

  float t=iTime*.35;

  float angle=snoise(uv*.7+t*.15)*3.14159;
  uv+=vec2(cos(angle),sin(angle))*.25;

  float n1=snoise(uv*1.5+vec2(t*.3,-t*.2))*.5;
  float n2=snoise(uv*3.+vec2(-t*.2,t*.3))*.25;
  float pattern=(n1+n2+1.)*.5;

  float bands=sin(pattern*14.-t*1.8);
  bands=smoothstep(.3,.7,bands);

  float detail=snoise(uv*8.+t*.5)*.5+.5;
  bands=mix(bands,bands+detail*.25,.2);

  float dist=length(uv);
  float disp=snoise(uv*2.+t*.6)*.25;
  float sphere=smoothstep(2.,0.,dist+disp);

  float fresnel=pow(1.-smoothstep(0.,1.6,dist),.8);
  float rim=pow(smoothstep(.3,1.4,dist)*.8,1.5)*fresnel;

  vec3 c1=vec3(.45,.45,.48);
  vec3 c2=vec3(.6,.6,.63);
  vec3 c3=vec3(.35,.35,.38);

  vec3 col=mix(c1,c2,bands);
  col=mix(col,c3,rim*.6);

  float intensity=bands*sphere*.35+fresnel*.06;
  col*=intensity;

  col*=smoothstep(2.2,.4,dist);

  float trail=snoise(uv*1.2-vec2(t*.4,t*.1))*.5+.5;
  col+=c1*trail*.02*sphere;

  gl_FragColor=vec4(col,1.);
}`;

function compileShader(gl: WebGLRenderingContext, type: number, src: string) {
  const s = gl.createShader(type)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  return s;
}

export default function FluidBackground() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const gl = canvas.getContext("webgl", { antialias: true, alpha: false });
    if (!gl) return;

    const prog = gl.createProgram()!;
    gl.attachShader(prog, compileShader(gl, gl.VERTEX_SHADER, VERTEX));
    gl.attachShader(prog, compileShader(gl, gl.FRAGMENT_SHADER, FRAGMENT));
    gl.linkProgram(prog);
    gl.useProgram(prog);

    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 1,-1, -1,1, -1,1, 1,-1, 1,1]), gl.STATIC_DRAW);

    const pos = gl.getAttribLocation(prog, "position");
    gl.enableVertexAttribArray(pos);
    gl.vertexAttribPointer(pos, 2, gl.FLOAT, false, 0, 0);

    const uTime = gl.getUniformLocation(prog, "iTime");
    const uRes = gl.getUniformLocation(prog, "iResolution");

    let raf = 0;
    const t0 = performance.now();

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      canvas.width = w * dpr;
      canvas.height = h * dpr;
      gl.viewport(0, 0, canvas.width, canvas.height);
    };
    resize();
    window.addEventListener("resize", resize);

    const loop = () => {
      const elapsed = (performance.now() - t0) / 1000;
      gl.uniform1f(uTime, elapsed);
      gl.uniform2f(uRes, canvas.width, canvas.height);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", resize);
    };
  }, []);

  return <canvas ref={canvasRef} className="fluid-background" />;
}
