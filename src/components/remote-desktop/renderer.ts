import type { RemoteDesktopFramePatch } from "@/lib/remoteDesktopFrame";

export interface RemoteDesktopRenderer {
  readonly mode: "webgl2" | "canvas2d";
  draw(patch: RemoteDesktopFramePatch): void;
  drawMany(patches: RemoteDesktopFramePatch[]): void;
  dispose(): void;
}

function ensureCanvasSize(
  canvas: HTMLCanvasElement,
  width: number,
  height: number,
) {
  if (canvas.width === width && canvas.height === height) return false;
  canvas.width = width;
  canvas.height = height;
  return true;
}

function copyPatchToRgba(
  patch: RemoteDesktopFramePatch,
  target: Uint8ClampedArray | Uint8Array,
) {
  const rowBytes = patch.width * 4;
  for (let row = 0; row < patch.height; row += 1) {
    const srcRow = row * patch.stride;
    const dstRow = row * rowBytes;
    if (patch.pixelFormat === "RGBA8888") {
      target.set(patch.payload.subarray(srcRow, srcRow + rowBytes), dstRow);
      continue;
    }
    for (
      let src = srcRow, dst = dstRow;
      src < srcRow + rowBytes;
      src += 4, dst += 4
    ) {
      target[dst] = patch.payload[src + 2] ?? 0;
      target[dst + 1] = patch.payload[src + 1] ?? 0;
      target[dst + 2] = patch.payload[src] ?? 0;
      target[dst + 3] = patch.payload[src + 3] ?? 255;
    }
  }
}

export class Canvas2dRemoteDesktopRenderer implements RemoteDesktopRenderer {
  readonly mode = "canvas2d";
  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly ctx: CanvasRenderingContext2D,
  ) {}

  draw(patch: RemoteDesktopFramePatch) {
    ensureCanvasSize(this.canvas, patch.desktopWidth, patch.desktopHeight);
    const imageData = this.ctx.createImageData(patch.width, patch.height);
    copyPatchToRgba(patch, imageData.data);
    this.ctx.putImageData(imageData, patch.x, patch.y);
  }

  drawMany(patches: RemoteDesktopFramePatch[]) {
    for (const patch of patches) {
      this.draw(patch);
    }
  }

  dispose() {}
}

export class WebGl2RemoteDesktopRenderer implements RemoteDesktopRenderer {
  readonly mode = "webgl2";
  private readonly program: WebGLProgram;
  private readonly texture: WebGLTexture;
  private readonly positionBuffer: WebGLBuffer;
  private readonly texCoordBuffer: WebGLBuffer;
  private textureWidth = 0;
  private textureHeight = 0;
  private disposed = false;

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly gl: WebGL2RenderingContext,
  ) {
    const shaders: WebGLShader[] = [];
    let program: WebGLProgram | null = null;
    let texture: WebGLTexture | null = null;
    let positionBuffer: WebGLBuffer | null = null;
    let texCoordBuffer: WebGLBuffer | null = null;
    try {
      const vertexShader = compileShader(
        gl,
        gl.VERTEX_SHADER,
        `#version 300 es
      in vec2 a_position;
      in vec2 a_texCoord;
      out vec2 v_texCoord;
      void main() {
        gl_Position = vec4(a_position, 0.0, 1.0);
        v_texCoord = a_texCoord;
      }`,
      );
      shaders.push(vertexShader);
      const fragmentShader = compileShader(
        gl,
        gl.FRAGMENT_SHADER,
        `#version 300 es
      precision mediump float;
      in vec2 v_texCoord;
      uniform sampler2D u_texture;
      out vec4 outColor;
      void main() {
        outColor = texture(u_texture, v_texCoord);
      }`,
      );

      shaders.push(fragmentShader);
      program = gl.createProgram();
      texture = gl.createTexture();
      positionBuffer = gl.createBuffer();
      texCoordBuffer = gl.createBuffer();
      if (!program || !texture || !positionBuffer || !texCoordBuffer) {
        throw new Error("Unable to create remote desktop WebGL resources");
      }

      gl.attachShader(program, vertexShader);
      gl.attachShader(program, fragmentShader);
      gl.linkProgram(program);
      if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
        const message =
          gl.getProgramInfoLog(program) ||
          "Unable to link remote desktop WebGL program";
        throw new Error(message);
      }

      this.program = program;
      this.texture = texture;
      this.positionBuffer = positionBuffer;
      this.texCoordBuffer = texCoordBuffer;

      activateWebGlProgram(gl, this.program);
      gl.bindTexture(gl.TEXTURE_2D, this.texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

      const positionLocation = gl.getAttribLocation(this.program, "a_position");
      gl.bindBuffer(gl.ARRAY_BUFFER, this.positionBuffer);
      gl.bufferData(
        gl.ARRAY_BUFFER,
        new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
        gl.STATIC_DRAW,
      );
      gl.enableVertexAttribArray(positionLocation);
      gl.vertexAttribPointer(positionLocation, 2, gl.FLOAT, false, 0, 0);

      const texCoordLocation = gl.getAttribLocation(this.program, "a_texCoord");
      gl.bindBuffer(gl.ARRAY_BUFFER, this.texCoordBuffer);
      gl.bufferData(
        gl.ARRAY_BUFFER,
        new Float32Array([0, 1, 1, 1, 0, 0, 1, 0]),
        gl.STATIC_DRAW,
      );
      gl.enableVertexAttribArray(texCoordLocation);
      gl.vertexAttribPointer(texCoordLocation, 2, gl.FLOAT, false, 0, 0);
    } catch (error) {
      if (texture) gl.deleteTexture(texture);
      if (positionBuffer) gl.deleteBuffer(positionBuffer);
      if (texCoordBuffer) gl.deleteBuffer(texCoordBuffer);
      if (program) gl.deleteProgram(program);
      throw error;
    } finally {
      for (const shader of shaders) gl.deleteShader(shader);
    }
  }

  draw(patch: RemoteDesktopFramePatch) {
    this.drawMany([patch]);
  }

  drawMany(patches: RemoteDesktopFramePatch[]) {
    if (patches.length === 0) return;
    const lastPatch = patches[patches.length - 1];
    if (!lastPatch) return;
    const desktopWidth = lastPatch.desktopWidth;
    const desktopHeight = lastPatch.desktopHeight;
    const renderPatches = patches.filter(
      (patch) =>
        patch.desktopWidth === desktopWidth &&
        patch.desktopHeight === desktopHeight,
    );
    ensureCanvasSize(this.canvas, desktopWidth, desktopHeight);
    const gl = this.gl;
    activateWebGlProgram(gl, this.program);
    gl.viewport(0, 0, desktopWidth, desktopHeight);
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    if (
      this.textureWidth !== desktopWidth ||
      this.textureHeight !== desktopHeight
    ) {
      gl.texImage2D(
        gl.TEXTURE_2D,
        0,
        gl.RGBA,
        desktopWidth,
        desktopHeight,
        0,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        null,
      );
      this.textureWidth = desktopWidth;
      this.textureHeight = desktopHeight;
    }

    for (const patch of renderPatches) {
      const rowBytes = patch.width * 4;
      const requiredBytes = rowBytes * patch.height;
      const canUploadDirectly =
        patch.pixelFormat === "RGBA8888" && patch.stride === rowBytes;
      const patchBytes = canUploadDirectly
        ? patch.payload.subarray(0, requiredBytes)
        : new Uint8Array(requiredBytes);
      if (!canUploadDirectly) copyPatchToRgba(patch, patchBytes);

      gl.texSubImage2D(
        gl.TEXTURE_2D,
        0,
        patch.x,
        patch.y,
        patch.width,
        patch.height,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        patchBytes,
      );
    }
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    const gl = this.gl;
    gl.deleteTexture(this.texture);
    gl.deleteBuffer(this.positionBuffer);
    gl.deleteBuffer(this.texCoordBuffer);
    gl.deleteProgram(this.program);
  }
}

function compileShader(
  gl: WebGL2RenderingContext,
  type: number,
  source: string,
) {
  const shader = gl.createShader(type);
  if (!shader) throw new Error("Unable to create remote desktop WebGL shader");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const message =
      gl.getShaderInfoLog(shader) ||
      "Unable to compile remote desktop WebGL shader";
    gl.deleteShader(shader);
    throw new Error(message);
  }
  return shader;
}

function activateWebGlProgram(
  gl: WebGL2RenderingContext,
  program: WebGLProgram,
) {
  const activateProgram = gl.useProgram.bind(gl);
  activateProgram(program);
}

export function createRemoteDesktopRenderer(
  canvas: HTMLCanvasElement,
  preferCanvas2d = false,
): RemoteDesktopRenderer | null {
  const gl = preferCanvas2d
    ? null
    : canvas.getContext("webgl2", {
        alpha: false,
        antialias: false,
        depth: false,
        stencil: false,
      });
  if (gl) {
    // A canvas that has acquired WebGL cannot acquire 2D. Let the surface
    // replace it if initialization fails rather than silently returning null.
    return new WebGl2RemoteDesktopRenderer(canvas, gl);
  }
  const ctx = canvas.getContext("2d", { alpha: false });
  return ctx ? new Canvas2dRemoteDesktopRenderer(canvas, ctx) : null;
}
