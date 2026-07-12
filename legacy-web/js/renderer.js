import { clamp, computeBounds, fitCamera, hexToRgba, sampleTrack } from './geo.js';

const TILE_URL = 'https://tile.openstreetmap.org/{z}/{x}/{y}.png';

function shader(gl, type, source) {
  const compiled = gl.createShader(type);
  gl.shaderSource(compiled, source);
  gl.compileShader(compiled);
  if (!gl.getShaderParameter(compiled, gl.COMPILE_STATUS)) {
    const message = gl.getShaderInfoLog(compiled);
    gl.deleteShader(compiled);
    throw new Error(`WebGL shader error: ${message}`);
  }
  return compiled;
}

function program(gl, vertexSource, fragmentSource) {
  const value = gl.createProgram();
  const vertex = shader(gl, gl.VERTEX_SHADER, vertexSource);
  const fragment = shader(gl, gl.FRAGMENT_SHADER, fragmentSource);
  gl.attachShader(value, vertex);
  gl.attachShader(value, fragment);
  gl.linkProgram(value);
  gl.deleteShader(vertex);
  gl.deleteShader(fragment);
  if (!gl.getProgramParameter(value, gl.LINK_STATUS)) {
    const message = gl.getProgramInfoLog(value);
    gl.deleteProgram(value);
    throw new Error(`WebGL program error: ${message}`);
  }
  return value;
}

const CAMERA_VERTEX = `#version 300 es
  in vec2 a_position;
  uniform vec2 u_center;
  uniform float u_scale;
  uniform vec2 u_resolution;
  vec2 project(vec2 position) {
    vec2 pixel = (position - u_center) * u_scale;
    return vec2(pixel.x * 2.0 / u_resolution.x, -pixel.y * 2.0 / u_resolution.y);
  }
`;

export class GpuMapRenderer {
  constructor(canvas, { onInvalidate = () => {} } = {}) {
    this.canvas = canvas;
    this.onInvalidate = onInvalidate;
    this.gl = canvas.getContext('webgl2', {
      alpha: true,
      antialias: true,
      depth: false,
      stencil: false,
      premultipliedAlpha: false,
      preserveDrawingBuffer: true,
      powerPreference: 'high-performance',
    });
    if (!this.gl) throw new Error('此瀏覽器或顯示卡不支援 WebGL2。');

    this.track = null;
    this.bounds = null;
    this.fit = null;
    this.manualCamera = null;
    this.tileCache = new Map();
    this.pairEnds = [];
    this.lastOptions = {};
    this.#createPrograms();
    this.#createBuffers();
    this.#configure();
  }

  #createPrograms() {
    const gl = this.gl;
    this.backgroundProgram = program(gl, `#version 300 es
      precision highp float;
      const vec2 POSITIONS[3] = vec2[3](vec2(-1.0,-1.0), vec2(3.0,-1.0), vec2(-1.0,3.0));
      void main() { gl_Position = vec4(POSITIONS[gl_VertexID], 0.0, 1.0); }
    `, `#version 300 es
      precision highp float;
      uniform vec2 u_resolution;
      uniform int u_style;
      out vec4 outColor;
      void main() {
        vec2 p = gl_FragCoord.xy;
        float minorX = 1.0 - step(1.0, mod(p.x, 24.0));
        float minorY = 1.0 - step(1.0, mod(p.y, 24.0));
        float majorX = 1.0 - step(1.0, mod(p.x, 96.0));
        float majorY = 1.0 - step(1.0, mod(p.y, 96.0));
        float grid = max(max(minorX, minorY) * 0.025, max(majorX, majorY) * 0.055);
        vec3 base = u_style == 1 ? vec3(0.90, 0.92, 0.93) : vec3(0.045, 0.064, 0.080);
        vec3 ink = u_style == 1 ? vec3(0.20, 0.25, 0.28) : vec3(0.58, 0.68, 0.74);
        outColor = vec4(mix(base, ink, grid), 1.0);
      }
    `);

    this.tileProgram = program(gl, `${CAMERA_VERTEX}
      in vec2 a_uv;
      out vec2 v_uv;
      void main() {
        gl_Position = vec4(project(a_position), 0.0, 1.0);
        v_uv = a_uv;
      }
    `, `#version 300 es
      precision highp float;
      uniform sampler2D u_texture;
      uniform int u_style;
      in vec2 v_uv;
      out vec4 outColor;
      void main() {
        vec3 source = texture(u_texture, v_uv).rgb;
        if (u_style == 0) {
          float luma = dot(source, vec3(0.299, 0.587, 0.114));
          vec3 inverted = mix(vec3(0.035, 0.052, 0.066), vec3(0.45, 0.51, 0.54), 1.0 - luma);
          vec3 tint = vec3(source.r * 0.06, source.g * 0.08, source.b * 0.10);
          outColor = vec4(inverted + tint, 1.0);
        } else {
          outColor = vec4(mix(source, vec3(0.94, 0.95, 0.95), 0.08), 1.0);
        }
      }
    `);

    this.lineProgram = program(gl, `${CAMERA_VERTEX}
      in vec2 a_normal;
      in float a_side;
      uniform float u_width;
      void main() {
        vec2 clip = project(a_position);
        vec2 offset = vec2(
          a_normal.x * a_side * u_width * 2.0 / u_resolution.x,
          -a_normal.y * a_side * u_width * 2.0 / u_resolution.y
        );
        gl_Position = vec4(clip + offset, 0.0, 1.0);
      }
    `, `#version 300 es
      precision highp float;
      uniform vec4 u_color;
      out vec4 outColor;
      void main() { outColor = u_color; }
    `);

    this.markerProgram = program(gl, `${CAMERA_VERTEX}
      uniform float u_size;
      void main() {
        gl_Position = vec4(project(a_position), 0.0, 1.0);
        gl_PointSize = u_size;
      }
    `, `#version 300 es
      precision highp float;
      uniform vec4 u_color;
      out vec4 outColor;
      void main() {
        float distanceFromCenter = length(gl_PointCoord - vec2(0.5));
        float outer = 1.0 - smoothstep(0.46, 0.50, distanceFromCenter);
        float inner = 1.0 - smoothstep(0.23, 0.29, distanceFromCenter);
        vec4 halo = vec4(u_color.rgb, u_color.a * 0.28);
        outColor = mix(halo, u_color, inner) * outer;
      }
    `);
  }

  #createBuffers() {
    const gl = this.gl;
    this.tileBuffer = gl.createBuffer();
    this.routeBuffer = gl.createBuffer();
    this.partialBuffer = gl.createBuffer();
    this.markerBuffer = gl.createBuffer();
  }

  #configure() {
    const gl = this.gl;
    gl.disable(gl.DEPTH_TEST);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
  }

  getGpuInfo() {
    const gl = this.gl;
    const extension = gl.getExtension('WEBGL_debug_renderer_info');
    const renderer = extension
      ? gl.getParameter(extension.UNMASKED_RENDERER_WEBGL)
      : gl.getParameter(gl.RENDERER);
    return {
      renderer: String(renderer).replace(/^ANGLE \(/, '').replace(/\)$/, ''),
      version: gl.getParameter(gl.VERSION),
      maxTextureSize: gl.getParameter(gl.MAX_TEXTURE_SIZE),
    };
  }

  setTrack(track) {
    this.track = track;
    this.bounds = track ? computeBounds(track.points) : null;
    this.#buildRouteMesh();
    this.fit = null;
    this.manualCamera = null;
  }

  #segmentVertices(a, b) {
    const dx = b.world.x - a.world.x;
    const dy = b.world.y - a.world.y;
    const length = Math.hypot(dx, dy) || 1;
    const nx = -dy / length;
    const ny = dx / length;
    const p1 = a.world;
    const p2 = b.world;
    return new Float32Array([
      p1.x, p1.y, nx, ny, -1,
      p2.x, p2.y, nx, ny, -1,
      p1.x, p1.y, nx, ny,  1,
      p1.x, p1.y, nx, ny,  1,
      p2.x, p2.y, nx, ny, -1,
      p2.x, p2.y, nx, ny,  1,
    ]);
  }

  #buildRouteMesh() {
    const gl = this.gl;
    const vertices = [];
    this.pairEnds = [];
    if (this.track) {
      for (let index = 0; index < this.track.points.length - 1; index += 1) {
        const a = this.track.points[index];
        const b = this.track.points[index + 1];
        if (a.segmentIndex !== b.segmentIndex) continue;
        vertices.push(...this.#segmentVertices(a, b));
        this.pairEnds.push(index + 1);
      }
    }
    gl.bindBuffer(gl.ARRAY_BUFFER, this.routeBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(vertices), gl.STATIC_DRAW);
  }

  resize(width, height) {
    const previousWidth = this.canvas.width;
    const targetWidth = Math.max(2, Math.round(width ?? this.canvas.clientWidth * Math.min(devicePixelRatio, 2)));
    const targetHeight = Math.max(2, Math.round(height ?? this.canvas.clientHeight * Math.min(devicePixelRatio, 2)));
    if (this.canvas.width !== targetWidth || this.canvas.height !== targetHeight) {
      if (this.manualCamera && previousWidth > 0) this.manualCamera.scale *= targetWidth / previousWidth;
      this.canvas.width = targetWidth;
      this.canvas.height = targetHeight;
      this.fit = null;
    }
    this.gl.viewport(0, 0, this.canvas.width, this.canvas.height);
  }

  #camera(sample, mode) {
    if (!this.fit) this.fit = fitCamera(this.bounds, this.canvas.width, this.canvas.height, 0.13);
    if (mode === 'free') {
      if (!this.manualCamera) {
        this.manualCamera = { center: { ...this.fit.center }, scale: this.fit.scale };
      }
      return this.manualCamera;
    }
    if (mode === 'follow' && sample) {
      return {
        center: sample.world,
        scale: clamp(this.fit.scale * 2.15, 256 * 2 ** 10, 256 * 2 ** 17),
      };
    }
    return this.fit;
  }

  beginFreeCamera(progress = 0, fromMode = 'fit') {
    if (!this.track) return;
    const sample = sampleTrack(this.track, progress);
    const current = this.#camera(sample, fromMode);
    this.manualCamera = { center: { ...current.center }, scale: current.scale };
  }

  clearManualCamera() {
    this.manualCamera = null;
  }

  resetCamera() {
    this.manualCamera = null;
    this.fit = null;
  }

  panByPixels(deltaX, deltaY) {
    if (!this.manualCamera) return;
    const ratioX = this.canvas.width / Math.max(1, this.canvas.clientWidth);
    const ratioY = this.canvas.height / Math.max(1, this.canvas.clientHeight);
    this.manualCamera.center.x = clamp(
      this.manualCamera.center.x - (deltaX * ratioX) / this.manualCamera.scale,
      -1,
      2,
    );
    this.manualCamera.center.y = clamp(
      this.manualCamera.center.y - (deltaY * ratioY) / this.manualCamera.scale,
      0,
      1,
    );
  }

  zoomAt(clientX, clientY, factor) {
    if (!this.manualCamera) return;
    const rect = this.canvas.getBoundingClientRect();
    const x = ((clientX - rect.left) / Math.max(1, rect.width)) * this.canvas.width;
    const y = ((clientY - rect.top) / Math.max(1, rect.height)) * this.canvas.height;
    const offsetX = x - this.canvas.width / 2;
    const offsetY = y - this.canvas.height / 2;
    const worldX = this.manualCamera.center.x + offsetX / this.manualCamera.scale;
    const worldY = this.manualCamera.center.y + offsetY / this.manualCamera.scale;
    const nextScale = clamp(this.manualCamera.scale * factor, 256 * 2, 256 * 2 ** 20);
    this.manualCamera.center.x = clamp(worldX - offsetX / nextScale, -1, 2);
    this.manualCamera.center.y = clamp(worldY - offsetY / nextScale, 0, 1);
    this.manualCamera.scale = nextScale;
  }

  zoomBy(factor) {
    if (!this.manualCamera) return;
    this.manualCamera.scale = clamp(this.manualCamera.scale * factor, 256 * 2, 256 * 2 ** 20);
  }

  #uniformCamera(programValue, camera) {
    const gl = this.gl;
    gl.uniform2f(gl.getUniformLocation(programValue, 'u_center'), camera.center.x, camera.center.y);
    gl.uniform1f(gl.getUniformLocation(programValue, 'u_scale'), camera.scale);
    gl.uniform2f(gl.getUniformLocation(programValue, 'u_resolution'), this.canvas.width, this.canvas.height);
  }

  #drawBackground(style) {
    if (style === 'transparent') return;
    const gl = this.gl;
    gl.useProgram(this.backgroundProgram);
    gl.uniform2f(gl.getUniformLocation(this.backgroundProgram, 'u_resolution'), this.canvas.width, this.canvas.height);
    gl.uniform1i(gl.getUniformLocation(this.backgroundProgram, 'u_style'), style === 'light' ? 1 : 0);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  #getTile(z, x, y) {
    const count = 2 ** z;
    const wrappedX = ((x % count) + count) % count;
    const key = `${z}/${wrappedX}/${y}`;
    let entry = this.tileCache.get(key);
    if (entry) {
      entry.lastUsed = performance.now();
      return entry;
    }

    const gl = this.gl;
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([20, 28, 35, 255]));

    let settle;
    const ready = new Promise((resolveReady) => { settle = resolveReady; });
    entry = { texture, loaded: false, failed: false, lastUsed: performance.now(), ready };
    this.tileCache.set(key, entry);
    const image = new Image();
    image.crossOrigin = 'anonymous';
    image.decoding = 'async';
    image.onload = () => {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
      entry.loaded = true;
      settle();
      this.onInvalidate();
    };
    image.onerror = () => { entry.failed = true; settle(); };
    image.src = TILE_URL.replace('{z}', z).replace('{x}', wrappedX).replace('{y}', y);
    return entry;
  }

  #drawTiles(camera, style) {
    if (style === 'transparent') return;
    const gl = this.gl;
    let zoom = clamp(Math.round(Math.log2(camera.scale / 256)), 1, 18);
    const visibleWidth = this.canvas.width / camera.scale;
    const visibleHeight = this.canvas.height / camera.scale;
    while (visibleWidth * visibleHeight * 4 ** zoom > 72 && zoom > 1) zoom -= 1;

    const count = 2 ** zoom;
    const xMin = Math.floor((camera.center.x - visibleWidth / 2) * count) - 1;
    const xMax = Math.ceil((camera.center.x + visibleWidth / 2) * count) + 1;
    const yMin = clamp(Math.floor((camera.center.y - visibleHeight / 2) * count) - 1, 0, count - 1);
    const yMax = clamp(Math.ceil((camera.center.y + visibleHeight / 2) * count) + 1, 0, count - 1);

    gl.useProgram(this.tileProgram);
    this.#uniformCamera(this.tileProgram, camera);
    gl.uniform1i(gl.getUniformLocation(this.tileProgram, 'u_texture'), 0);
    gl.uniform1i(gl.getUniformLocation(this.tileProgram, 'u_style'), style === 'light' ? 1 : 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.tileBuffer);
    const position = gl.getAttribLocation(this.tileProgram, 'a_position');
    const uv = gl.getAttribLocation(this.tileProgram, 'a_uv');
    gl.enableVertexAttribArray(position);
    gl.enableVertexAttribArray(uv);
    gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 16, 0);
    gl.vertexAttribPointer(uv, 2, gl.FLOAT, false, 16, 8);

    for (let y = yMin; y <= yMax; y += 1) {
      for (let x = xMin; x <= xMax; x += 1) {
        const tile = this.#getTile(zoom, x, y);
        if (!tile.loaded) continue;
        const x0 = x / count;
        const y0 = y / count;
        const x1 = (x + 1) / count;
        const y1 = (y + 1) / count;
        gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([
          x0, y0, 0, 1, x1, y0, 1, 1, x0, y1, 0, 0,
          x0, y1, 0, 0, x1, y0, 1, 1, x1, y1, 1, 0,
        ]), gl.STREAM_DRAW);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, tile.texture);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
      }
    }
    this.#trimTileCache();
  }

  #trimTileCache() {
    if (this.tileCache.size <= 180) return;
    const gl = this.gl;
    const oldest = [...this.tileCache.entries()].sort((a, b) => a[1].lastUsed - b[1].lastUsed).slice(0, this.tileCache.size - 140);
    for (const [key, entry] of oldest) {
      gl.deleteTexture(entry.texture);
      this.tileCache.delete(key);
    }
  }

  #bindLineBuffer(buffer) {
    const gl = this.gl;
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    const position = gl.getAttribLocation(this.lineProgram, 'a_position');
    const normal = gl.getAttribLocation(this.lineProgram, 'a_normal');
    const side = gl.getAttribLocation(this.lineProgram, 'a_side');
    gl.enableVertexAttribArray(position);
    gl.enableVertexAttribArray(normal);
    gl.enableVertexAttribArray(side);
    gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 20, 0);
    gl.vertexAttribPointer(normal, 2, gl.FLOAT, false, 20, 8);
    gl.vertexAttribPointer(side, 1, gl.FLOAT, false, 20, 16);
  }

  #setLineUniforms(camera, width, color) {
    const gl = this.gl;
    this.#uniformCamera(this.lineProgram, camera);
    gl.uniform1f(gl.getUniformLocation(this.lineProgram, 'u_width'), width / 2);
    gl.uniform4fv(gl.getUniformLocation(this.lineProgram, 'u_color'), color);
  }

  #completedPairCount(pointIndex) {
    let low = 0;
    let high = this.pairEnds.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (this.pairEnds[mid] <= pointIndex) low = mid + 1;
      else high = mid;
    }
    return low;
  }

  #drawRoute(camera, sample, options) {
    if (!this.track || !this.pairEnds.length) return;
    const gl = this.gl;
    gl.useProgram(this.lineProgram);
    this.#bindLineBuffer(this.routeBuffer);

    const routeColor = hexToRgba(options.routeColor ?? '#ff5d3b', 1);
    const remaining = options.mapStyle === 'light' ? [0.15, 0.19, 0.22, 0.62] : [0.84, 0.90, 0.94, 0.52];
    const width = Number(options.lineWidth ?? 8) * (this.canvas.width / Math.max(1, this.canvas.clientWidth));
    this.#setLineUniforms(camera, width + 2, options.mapStyle === 'light' ? [1, 1, 1, .58] : [0, 0, 0, .42]);
    gl.drawArrays(gl.TRIANGLES, 0, this.pairEnds.length * 6);
    this.#setLineUniforms(camera, width, remaining);
    gl.drawArrays(gl.TRIANGLES, 0, this.pairEnds.length * 6);

    const completed = this.#completedPairCount(sample.index);
    if (completed > 0) {
      this.#setLineUniforms(camera, width, routeColor);
      gl.drawArrays(gl.TRIANGLES, 0, completed * 6);
    }

    const a = this.track.points[sample.index];
    const b = this.track.points[sample.index + 1];
    if (a && b && a.segmentIndex === b.segmentIndex && sample.fraction > 0) {
      const partialPoint = { world: sample.world };
      const data = this.#segmentVertices(a, partialPoint);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.partialBuffer);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STREAM_DRAW);
      this.#bindLineBuffer(this.partialBuffer);
      this.#setLineUniforms(camera, width, routeColor);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    }
  }

  #drawMarker(camera, sample, options) {
    if (!sample) return;
    const gl = this.gl;
    gl.useProgram(this.markerProgram);
    this.#uniformCamera(this.markerProgram, camera);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.markerBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([sample.world.x, sample.world.y]), gl.STREAM_DRAW);
    const position = gl.getAttribLocation(this.markerProgram, 'a_position');
    gl.enableVertexAttribArray(position);
    gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
    const pixelRatio = this.canvas.width / Math.max(1, this.canvas.clientWidth);
    gl.uniform1f(gl.getUniformLocation(this.markerProgram, 'u_size'), clamp(34 * pixelRatio, 24, 96));
    gl.uniform4fv(gl.getUniformLocation(this.markerProgram, 'u_color'), hexToRgba(options.markerColor ?? '#fff3d6'));
    gl.drawArrays(gl.POINTS, 0, 1);
  }

  render(progress = 0, options = {}) {
    this.lastOptions = options;
    const gl = this.gl;
    const mapStyle = options.mapStyle ?? 'dark';
    const sample = this.track ? sampleTrack(this.track, progress) : null;
    const camera = this.#camera(sample, options.cameraMode ?? 'fit');
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    if (mapStyle === 'transparent') gl.clearColor(0, 0, 0, 0);
    else if (mapStyle === 'light') gl.clearColor(.9, .92, .93, 1);
    else gl.clearColor(.035, .05, .064, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    this.#drawBackground(mapStyle);
    this.#drawTiles(camera, mapStyle);
    this.#drawRoute(camera, sample, options);
    this.#drawMarker(camera, sample, options);
    return sample;
  }

  async waitForTiles(timeoutMs = 4_000) {
    const pending = [...this.tileCache.values()].filter((entry) => !entry.loaded && !entry.failed).map((entry) => entry.ready);
    if (!pending.length) return;
    await Promise.race([
      Promise.allSettled(pending),
      new Promise((resolveWait) => setTimeout(resolveWait, timeoutMs)),
    ]);
  }

  destroy() {
    const gl = this.gl;
    for (const entry of this.tileCache.values()) gl.deleteTexture(entry.texture);
    [this.tileBuffer, this.routeBuffer, this.partialBuffer, this.markerBuffer].forEach((buffer) => gl.deleteBuffer(buffer));
    [this.backgroundProgram, this.tileProgram, this.lineProgram, this.markerProgram].forEach((item) => gl.deleteProgram(item));
  }
}
