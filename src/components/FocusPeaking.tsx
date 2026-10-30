import { useEffect, useRef } from "react";
import { peakRgb, peakSize, peakThreshold, type PeakColor } from "../lib/composition";

/**
 * Focus peaking: the frame's sharp detail marked in colour, over the
 * photo, so whether the eye or the ear is in focus shows at a glance.
 *
 * Measured on the GPU into a canvas laid exactly over the image, which zoom
 * and pan move with the image as one — so a pan costs nothing, and a zoom
 * only a redraw once it settles. The measure is taken at the resolution the
 * photo is shown at, as a camera's viewfinder does: fitted, at screen pixels,
 * so marks don't dissolve as a 24 MP frame is shrunk to fit; at 1:1 with the
 * full-resolution source, at sensor pixels. The source is uploaded once and
 * read at the right scale through its mipmaps (WebGL 2; WebGL 1, which lacks
 * them for most sizes, measures at the source's own size).
 *
 * What is marked is fine detail, not edges: how far a pixel's brightness
 * stands from its eight neighbours', as a share of the contrast around it
 * (see `peakThreshold`). A sharp edge concentrates its contrast in a pixel;
 * the same edge out of focus spreads it, however much contrast it has — which
 * is what a gradient-strength measure gets wrong, marking every contrasty but
 * soft outline.
 *
 * The source is read again with CORS, as WebGL requires: the image on screen
 * is left as it was, so peaking failing can never take the photo with it.
 */

const VERTEX = `
attribute vec2 a_pos;
varying vec2 v_uv;
void main() {
  v_uv = vec2(a_pos.x * 0.5 + 0.5, 0.5 - a_pos.y * 0.5);
  gl_Position = vec4(a_pos, 0.0, 1.0);
}`;

const FRAGMENT = `
precision mediump float;
uniform sampler2D u_img;
uniform vec2 u_px;
uniform float u_share;
uniform float u_floor;
uniform vec3 u_color;
varying vec2 v_uv;
float lum(vec2 o) {
  return dot(texture2D(u_img, v_uv + o * u_px).rgb, vec3(0.2126, 0.7152, 0.0722));
}
void main() {
  float centre = lum(vec2(0.0));
  float ring = 0.0;
  float lo = centre;
  float hi = centre;
  for (int y = -2; y <= 2; y++) {
    for (int x = -2; x <= 2; x++) {
      if (x == 0 && y == 0) continue;
      float v = lum(vec2(float(x), float(y)));
      lo = min(lo, v);
      hi = max(hi, v);
      if (abs(float(x)) <= 1.0 && abs(float(y)) <= 1.0) ring += v;
    }
  }
  float range = hi - lo;
  float share = abs(centre - ring / 8.0) / max(range, 1e-3);
  float a = smoothstep(u_share, u_share * 1.2, share) * smoothstep(u_floor, u_floor * 1.5, range);
  gl_FragColor = vec4(u_color * a, a);
}`;

/** How long a zoom must settle before the marks are measured again. */
const SETTLE_MS = 140;

type Ctx = WebGLRenderingContext | WebGL2RenderingContext;

interface Gl {
  gl: Ctx;
  program: WebGLProgram;
  mipmaps: boolean;
  texture: WebGLTexture | null;
  /** The source's size as uploaded. */
  size: { width: number; height: number } | null;
}

function compile(gl: Ctx): WebGLProgram | null {
  const shader = (type: number, source: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, source);
    gl.compileShader(s);
    return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null;
  };
  const vs = shader(gl.VERTEX_SHADER, VERTEX);
  const fs = shader(gl.FRAGMENT_SHADER, FRAGMENT);
  if (!vs || !fs) return null;
  const program = gl.createProgram()!;
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) return null;
  gl.useProgram(program);
  const buffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  const pos = gl.getAttribLocation(program, "a_pos");
  gl.enableVertexAttribArray(pos);
  gl.vertexAttribPointer(pos, 2, gl.FLOAT, false, 0, 0);
  return program;
}

export type PeakingStatus = "loading" | "ready" | "failed";

export default function FocusPeaking({
  src,
  color,
  sensitivity,
  shownWidth,
  onStatus,
}: {
  src: string | undefined;
  color: PeakColor;
  sensitivity: number;
  /** How many device pixels wide the photo is on screen, zoom included. */
  shownWidth: number;
  onStatus?: (status: PeakingStatus) => void;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const glRef = useRef<Gl | null>(null);
  const status = useRef(onStatus);
  status.current = onStatus;
  const look = useRef({ color, sensitivity, shownWidth });
  look.current = { color, sensitivity, shownWidth };

  const draw = () => {
    const g = glRef.current;
    const canvas = canvasRef.current;
    if (!g?.texture || !g.size || !canvas) return;
    const { gl, program, size } = g;
    // Measured at the size shown, never beyond the source's own.
    const k = g.mipmaps ? Math.min(1, Math.max(64, look.current.shownWidth) / size.width) : 1;
    const out = {
      width: Math.max(1, Math.round(size.width * k)),
      height: Math.max(1, Math.round(size.height * k)),
    };
    if (canvas.width !== out.width || canvas.height !== out.height) {
      canvas.width = out.width;
      canvas.height = out.height;
    }
    gl.viewport(0, 0, out.width, out.height);
    gl.uniform2f(gl.getUniformLocation(program, "u_px"), 1 / out.width, 1 / out.height);
    const { share, floor } = peakThreshold(look.current.sensitivity);
    gl.uniform1f(gl.getUniformLocation(program, "u_share"), share);
    gl.uniform1f(gl.getUniformLocation(program, "u_floor"), floor);
    gl.uniform3fv(gl.getUniformLocation(program, "u_color"), peakRgb(look.current.color));
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  };

  // One canvas and context for the life of the overlay, given back when it
  // goes: a Compare of four frames must not leave four behind on every set.
  // The canvas is made here rather than rendered, so a context given back is
  // never asked for again from the same canvas — which returns the lost one
  // (React runs this twice in development, and the second run failed).
  useEffect(() => {
    const canvas = document.createElement("canvas");
    canvas.dataset.peaking = "";
    canvas.className = "pointer-events-none absolute inset-0 h-full w-full";
    hostRef.current?.appendChild(canvas);
    canvasRef.current = canvas;
    const options = { premultipliedAlpha: true, antialias: false };
    const gl2 = (canvas?.getContext("webgl2", options) ?? null) as WebGL2RenderingContext | null;
    const gl: Ctx | null = gl2 ?? ((canvas?.getContext("webgl", options) ?? null) as WebGLRenderingContext | null);
    const program = gl ? compile(gl) : null;
    glRef.current = gl && program ? { gl, program, mipmaps: gl2 !== null, texture: null, size: null } : null;
    return () => {
      const g = glRef.current;
      glRef.current = null;
      canvasRef.current = null;
      canvas.remove();
      if (!g) return;
      if (g.texture) g.gl.deleteTexture(g.texture);
      g.gl.getExtension("WEBGL_lose_context")?.loseContext();
    };
  }, []);

  useEffect(() => {
    const g = glRef.current;
    if (!src) return;
    if (!g) {
      status.current?.("failed");
      return;
    }
    let live = true;
    status.current?.("loading");
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => {
      if (!live || glRef.current !== g) return;
      const { gl } = g;
      const max = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number;
      const size = peakSize(img.naturalWidth, img.naturalHeight, Math.min(8192, max));
      // Too big for a texture: reduced on a 2D canvas first.
      let source: TexImageSource = img;
      if (size.width !== img.naturalWidth) {
        const c = document.createElement("canvas");
        c.width = size.width;
        c.height = size.height;
        c.getContext("2d")!.drawImage(img, 0, 0, size.width, size.height);
        source = c;
      }
      if (g.texture) gl.deleteTexture(g.texture);
      const texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(
        gl.TEXTURE_2D,
        gl.TEXTURE_MIN_FILTER,
        g.mipmaps ? gl.LINEAR_MIPMAP_LINEAR : gl.NEAREST,
      );
      try {
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB, gl.RGB, gl.UNSIGNED_BYTE, source);
        if (g.mipmaps) gl.generateMipmap(gl.TEXTURE_2D);
      } catch {
        // A source without CORS taints the upload; the photo itself is fine.
        status.current?.("failed");
        return;
      }
      g.texture = texture;
      g.size = size;
      draw();
      status.current?.("ready");
    };
    img.onerror = () => {
      if (live) status.current?.("failed");
    };
    img.src = src;
    return () => {
      live = false;
    };
    // `draw` reads everything it needs from refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [src]);

  // A new look redraws at once; a zoom, once it settles.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(draw, [color, sensitivity]);
  useEffect(() => {
    const t = window.setTimeout(draw, SETTLE_MS);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shownWidth]);

  return <div ref={hostRef} className="pointer-events-none absolute inset-0" />;
}
