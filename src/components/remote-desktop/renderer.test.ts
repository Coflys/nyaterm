import { describe, expect, it, vi } from "vitest";
import {
  Canvas2dRemoteDesktopRenderer,
  createRemoteDesktopRenderer,
  WebGl2RemoteDesktopRenderer,
} from "./renderer";

function patch(pixelFormat: "RGBA8888" | "BGRA8888" = "RGBA8888") {
  return {
    sequence: 1n,
    desktopWidth: 4,
    desktopHeight: 3,
    x: 1,
    y: 1,
    width: 1,
    height: 1,
    stride: 4,
    pixelFormat,
    payload: new Uint8Array([1, 2, 3, 255]),
  } as const;
}

describe("remote desktop renderer", () => {
  it("falls back to Canvas2D when WebGL2 is unavailable", () => {
    const imageData = { data: new Uint8ClampedArray(4) } as ImageData;
    const context = {
      createImageData: vi.fn(() => imageData),
      putImageData: vi.fn(),
    } as unknown as CanvasRenderingContext2D;
    const canvas = document.createElement("canvas");
    vi.spyOn(canvas, "getContext").mockImplementation((type) =>
      type === "2d" ? context : null,
    );

    const renderer = createRemoteDesktopRenderer(canvas);
    expect(renderer).toBeInstanceOf(Canvas2dRemoteDesktopRenderer);

    renderer?.draw(patch("BGRA8888"));
    expect(canvas.width).toBe(4);
    expect(canvas.height).toBe(3);
    expect([...imageData.data]).toEqual([3, 2, 1, 255]);
    expect(context.putImageData).toHaveBeenCalledWith(imageData, 1, 1);
  });

  it("uploads a WebGL batch before issuing one draw call", () => {
    const gl = createWebGlMock();
    const canvas = document.createElement("canvas");
    const first = patch();
    const second = {
      ...patch(),
      sequence: 2n,
      x: 2,
      payload: new Uint8Array([4, 5, 6, 255]),
    };
    const renderer = new WebGl2RemoteDesktopRenderer(
      canvas,
      gl as unknown as WebGL2RenderingContext,
    );

    renderer.drawMany([first, second]);

    expect(canvas.width).toBe(4);
    expect(canvas.height).toBe(3);
    expect(gl.texSubImage2D).toHaveBeenCalledTimes(2);
    expect(gl.drawArrays).toHaveBeenCalledTimes(1);
  });

  it("returns null when neither rendering context is available", () => {
    const canvas = document.createElement("canvas");
    vi.spyOn(canvas, "getContext").mockReturnValue(null);

    expect(createRemoteDesktopRenderer(canvas)).toBeNull();
  });

  it("allocates texture storage even if a replacement renderer inherits matching canvas dimensions", () => {
    const canvas = document.createElement("canvas");
    canvas.width = 4;
    canvas.height = 3;
    const gl = createWebGlMock();
    const renderer = new WebGl2RemoteDesktopRenderer(
      canvas,
      gl as unknown as WebGL2RenderingContext,
    );
    renderer.draw(patch());
    renderer.draw(patch());
    expect(gl.texImage2D).toHaveBeenCalledOnce();
    renderer.draw({ ...patch(), desktopWidth: 8 });
    expect(gl.texImage2D).toHaveBeenCalledTimes(2);
    renderer.dispose();
    renderer.dispose();
    expect(gl.deleteTexture).toHaveBeenCalledOnce();
  });

  it("cleans all resources if shader linking fails instead of trying 2D on a WebGL canvas", () => {
    const gl = createWebGlMock();
    gl.getProgramParameter.mockReturnValue(false);
    const canvas = document.createElement("canvas");
    const getContext = vi
      .spyOn(canvas, "getContext")
      .mockReturnValue(gl as unknown as WebGL2RenderingContext);
    expect(() => createRemoteDesktopRenderer(canvas)).toThrow();
    expect(getContext).toHaveBeenCalledOnce();
    expect(gl.deleteShader).toHaveBeenCalledTimes(2);
    expect(gl.deleteBuffer).toHaveBeenCalledTimes(2);
    expect(gl.deleteTexture).toHaveBeenCalledOnce();
    expect(gl.deleteProgram).toHaveBeenCalledOnce();
  });

  it("cleans the first shader if the fragment shader fails to compile", () => {
    const gl = createWebGlMock();
    gl.getShaderParameter.mockReturnValueOnce(true).mockReturnValueOnce(false);
    expect(
      () =>
        new WebGl2RemoteDesktopRenderer(
          document.createElement("canvas"),
          gl as unknown as WebGL2RenderingContext,
        ),
    ).toThrow();
    expect(gl.deleteShader).toHaveBeenCalledTimes(2);
  });

  it("preserves the pixels of a recovered snapshot followed by incremental patches", () => {
    const pixels = new Uint8Array(8);
    const context = {
      createImageData: (width: number, height: number) => ({
        data: new Uint8ClampedArray(width * height * 4),
      }),
      putImageData: (image: ImageData, x: number) =>
        pixels.set(image.data, x * 4),
    } as unknown as CanvasRenderingContext2D;
    const renderer = new Canvas2dRemoteDesktopRenderer(
      document.createElement("canvas"),
      context,
    );
    const snapshot = {
      ...patch(),
      desktopWidth: 2,
      desktopHeight: 1,
      x: 0,
      y: 0,
      width: 2,
      height: 1,
      stride: 8,
      payload: new Uint8Array([1, 2, 3, 255, 4, 5, 6, 255]),
    };
    renderer.drawMany([
      snapshot,
      {
        ...snapshot,
        sequence: 2n,
        x: 1,
        width: 1,
        stride: 4,
        payload: new Uint8Array([7, 8, 9, 255]),
      },
    ]);
    expect([...pixels]).toEqual([1, 2, 3, 255, 7, 8, 9, 255]);
  });
});

function createWebGlMock() {
  return {
    ARRAY_BUFFER: 0x8892,
    CLAMP_TO_EDGE: 0x812f,
    COMPILE_STATUS: 0x8b81,
    DEPTH: 0x1801,
    FLOAT: 0x1406,
    FRAGMENT_SHADER: 0x8b30,
    LINK_STATUS: 0x8b82,
    NEAREST: 0x2600,
    RGBA: 0x1908,
    STATIC_DRAW: 0x88e4,
    TEXTURE_2D: 0x0de1,
    TEXTURE_MAG_FILTER: 0x2800,
    TEXTURE_MIN_FILTER: 0x2801,
    TEXTURE_WRAP_S: 0x2802,
    TEXTURE_WRAP_T: 0x2803,
    TRIANGLE_STRIP: 0x0005,
    UNSIGNED_BYTE: 0x1401,
    VERTEX_SHADER: 0x8b31,
    attachShader: vi.fn(),
    bindBuffer: vi.fn(),
    bindTexture: vi.fn(),
    bufferData: vi.fn(),
    compileShader: vi.fn(),
    createBuffer: vi.fn(() => ({})),
    createProgram: vi.fn(() => ({})),
    createShader: vi.fn(() => ({})),
    createTexture: vi.fn(() => ({})),
    deleteBuffer: vi.fn(),
    deleteProgram: vi.fn(),
    deleteShader: vi.fn(),
    deleteTexture: vi.fn(),
    drawArrays: vi.fn(),
    enableVertexAttribArray: vi.fn(),
    getAttribLocation: vi.fn(() => 0),
    getProgramInfoLog: vi.fn(() => ""),
    getProgramParameter: vi.fn(() => true),
    getShaderInfoLog: vi.fn(() => ""),
    getShaderParameter: vi.fn(() => true),
    linkProgram: vi.fn(),
    shaderSource: vi.fn(),
    texImage2D: vi.fn(),
    texParameteri: vi.fn(),
    texSubImage2D: vi.fn(),
    useProgram: vi.fn(),
    vertexAttribPointer: vi.fn(),
    viewport: vi.fn(),
  };
}
