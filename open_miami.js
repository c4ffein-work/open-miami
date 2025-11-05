// Custom WebAssembly loader for Open Miami
// Written from scratch - no external dependencies

(function() {
    'use strict';

    // Get the canvas element
    const canvas = document.getElementById('glcanvas');
    if (!canvas) {
        console.error('Canvas element #glcanvas not found');
        return;
    }

    // Global state
    let gl = null;
    let wasmMemory = null;
    let wasmExports = null;
    let animationId = null;

    // Initialize WebGL context
    function initWebGL() {
        gl = canvas.getContext('webgl2') || canvas.getContext('webgl');
        if (!gl) {
            alert('Unable to initialize WebGL. Your browser may not support it.');
            return false;
        }
        return true;
    }

    // Helper: Read UTF-8 string from WASM memory
    function readString(ptr, len) {
        const bytes = new Uint8Array(wasmMemory.buffer, ptr, len);
        return new TextDecoder('utf-8').decode(bytes);
    }

    // Helper: Write UTF-8 string to WASM memory
    function writeString(str, ptr, maxLen) {
        const bytes = new TextEncoder().encode(str);
        const dest = new Uint8Array(wasmMemory.buffer, ptr, maxLen);
        const copyLen = Math.min(bytes.length, maxLen);
        for (let i = 0; i < copyLen; i++) {
            dest[i] = bytes[i];
        }
        return copyLen;
    }

    // Helper: Get typed array view of WASM memory
    function getMemoryView(ptr, type, count) {
        return new type(wasmMemory.buffer, ptr, count);
    }

    // WebGL resource tracking
    const resources = {
        textures: [null],
        buffers: [null],
        programs: [null],
        shaders: [null],
        framebuffers: [null],
        renderbuffers: [null],
        vaos: [null],
        uniforms: [null],
        nextId: 1
    };

    function allocResource(array) {
        const id = resources.nextId++;
        array[id] = null;
        return id;
    }

    // Import object with all the functions the WASM module expects
    const importObject = {
        env: {
            // Console logging
            console_log: (ptr) => console.log(readString(ptr, 1024)),
            console_error: (ptr) => console.error(readString(ptr, 1024)),
            console_warn: (ptr) => console.warn(readString(ptr, 1024)),
            console_debug: (ptr) => console.debug(readString(ptr, 1024)),
            console_info: (ptr) => console.info(readString(ptr, 1024)),

            // Time
            now: () => Date.now() / 1000.0,

            // Random
            rand: () => Math.floor(Math.random() * 0x7FFFFFFF),

            // Canvas dimensions
            canvas_width: () => canvas.width,
            canvas_height: () => canvas.height,

            // DPI scaling
            dpi_scale: () => window.devicePixelRatio || 1.0,

            // WebGL initialization
            init_webgl: (version) => {
                return initWebGL() ? 1 : 0;
            },

            // Basic WebGL functions
            glClearColor: (r, g, b, a) => gl.clearColor(r, g, b, a),
            glClear: (mask) => gl.clear(mask),
            glViewport: (x, y, w, h) => gl.viewport(x, y, w, h),
            glEnable: (cap) => gl.enable(cap),
            glDisable: (cap) => gl.disable(cap),
            glBlendFunc: (sfactor, dfactor) => gl.blendFunc(sfactor, dfactor),
            glDepthFunc: (func) => gl.depthFunc(func),
            glClearDepthf: (depth) => gl.clearDepth(depth),

            // Texture functions
            glGenTextures: (n, ptr) => {
                const ids = getMemoryView(ptr, Uint32Array, n);
                for (let i = 0; i < n; i++) {
                    const tex = gl.createTexture();
                    const id = allocResource(resources.textures);
                    resources.textures[id] = tex;
                    ids[i] = id;
                }
            },
            glBindTexture: (target, id) => {
                gl.bindTexture(target, resources.textures[id]);
            },
            glTexImage2D: (target, level, internalformat, width, height, border, format, type, ptr) => {
                const data = ptr ? new Uint8Array(wasmMemory.buffer, ptr, width * height * 4) : null;
                gl.texImage2D(target, level, internalformat, width, height, border, format, type, data);
            },
            glTexParameteri: (target, pname, param) => {
                gl.texParameteri(target, pname, param);
            },
            glActiveTexture: (texture) => gl.activeTexture(texture),

            // Buffer functions
            glGenBuffers: (n, ptr) => {
                const ids = getMemoryView(ptr, Uint32Array, n);
                for (let i = 0; i < n; i++) {
                    const buf = gl.createBuffer();
                    const id = allocResource(resources.buffers);
                    resources.buffers[id] = buf;
                    ids[i] = id;
                }
            },
            glBindBuffer: (target, id) => {
                gl.bindBuffer(target, resources.buffers[id]);
            },
            glBufferData: (target, size, ptr, usage) => {
                const data = ptr ? new Uint8Array(wasmMemory.buffer, ptr, size) : size;
                gl.bufferData(target, data, usage);
            },
            glBufferSubData: (target, offset, size, ptr) => {
                const data = new Uint8Array(wasmMemory.buffer, ptr, size);
                gl.bufferSubData(target, offset, data);
            },

            // Shader and program functions
            glCreateShader: (type) => {
                const shader = gl.createShader(type);
                const id = allocResource(resources.shaders);
                resources.shaders[id] = shader;
                return id;
            },
            glShaderSource: (id, count, stringsPtr, lengthsPtr) => {
                const strings = getMemoryView(stringsPtr, Uint32Array, count);
                let source = '';
                for (let i = 0; i < count; i++) {
                    const strPtr = strings[i];
                    const len = lengthsPtr ? getMemoryView(lengthsPtr, Int32Array, count)[i] : 1000;
                    source += readString(strPtr, len);
                }
                gl.shaderSource(resources.shaders[id], source);
            },
            glCompileShader: (id) => gl.compileShader(resources.shaders[id]),
            glGetShaderiv: (id, pname, ptr) => {
                const result = gl.getShaderParameter(resources.shaders[id], pname);
                getMemoryView(ptr, Int32Array, 1)[0] = result;
            },
            glGetShaderInfoLog: (id, maxLen, lenPtr, infoPtr) => {
                const log = gl.getShaderInfoLog(resources.shaders[id]) || '';
                const len = writeString(log, infoPtr, maxLen);
                if (lenPtr) getMemoryView(lenPtr, Int32Array, 1)[0] = len;
            },
            glCreateProgram: () => {
                const program = gl.createProgram();
                const id = allocResource(resources.programs);
                resources.programs[id] = program;
                return id;
            },
            glAttachShader: (progId, shaderId) => {
                gl.attachShader(resources.programs[progId], resources.shaders[shaderId]);
            },
            glLinkProgram: (id) => gl.linkProgram(resources.programs[id]),
            glGetProgramiv: (id, pname, ptr) => {
                const result = gl.getProgramParameter(resources.programs[id], pname);
                getMemoryView(ptr, Int32Array, 1)[0] = result;
            },
            glUseProgram: (id) => gl.useProgram(resources.programs[id]),
            glDeleteShader: (id) => {
                if (resources.shaders[id]) {
                    gl.deleteShader(resources.shaders[id]);
                    resources.shaders[id] = null;
                }
            },

            // Attribute and uniform functions
            glGetAttribLocation: (progId, namePtr) => {
                const name = readString(namePtr, 256);
                return gl.getAttribLocation(resources.programs[progId], name);
            },
            glGetUniformLocation: (progId, namePtr) => {
                const name = readString(namePtr, 256);
                const loc = gl.getUniformLocation(resources.programs[progId], name);
                if (!loc) return -1;
                const id = allocResource(resources.uniforms);
                resources.uniforms[id] = loc;
                return id;
            },
            glEnableVertexAttribArray: (index) => gl.enableVertexAttribArray(index),
            glDisableVertexAttribArray: (index) => gl.disableVertexAttribArray(index),
            glVertexAttribPointer: (index, size, type, normalized, stride, offset) => {
                gl.vertexAttribPointer(index, size, type, normalized, stride, offset);
            },
            glUniform1f: (loc, v0) => gl.uniform1f(resources.uniforms[loc], v0),
            glUniform1i: (loc, v0) => gl.uniform1i(resources.uniforms[loc], v0),
            glUniform2f: (loc, v0, v1) => gl.uniform2f(resources.uniforms[loc], v0, v1),
            glUniform3f: (loc, v0, v1, v2) => gl.uniform3f(resources.uniforms[loc], v0, v1, v2),
            glUniform4f: (loc, v0, v1, v2, v3) => gl.uniform4f(resources.uniforms[loc], v0, v1, v2, v3),
            glUniform1fv: (loc, count, ptr) => {
                const data = getMemoryView(ptr, Float32Array, count);
                gl.uniform1fv(resources.uniforms[loc], data);
            },
            glUniform2fv: (loc, count, ptr) => {
                const data = getMemoryView(ptr, Float32Array, count * 2);
                gl.uniform2fv(resources.uniforms[loc], data);
            },
            glUniform3fv: (loc, count, ptr) => {
                const data = getMemoryView(ptr, Float32Array, count * 3);
                gl.uniform3fv(resources.uniforms[loc], data);
            },
            glUniform4fv: (loc, count, ptr) => {
                const data = getMemoryView(ptr, Float32Array, count * 4);
                gl.uniform4fv(resources.uniforms[loc], data);
            },
            glUniformMatrix4fv: (loc, count, transpose, ptr) => {
                const data = getMemoryView(ptr, Float32Array, count * 16);
                gl.uniformMatrix4fv(resources.uniforms[loc], transpose, data);
            },

            // Drawing
            glDrawArrays: (mode, first, count) => gl.drawArrays(mode, first, count),
            glDrawElements: (mode, count, type, offset) => gl.drawElements(mode, count, type, offset),

            // Vertex Array Objects (WebGL2 or extension)
            glGenVertexArrays: (n, ptr) => {
                const ids = getMemoryView(ptr, Uint32Array, n);
                for (let i = 0; i < n; i++) {
                    const vao = gl.createVertexArray ? gl.createVertexArray() : null;
                    const id = allocResource(resources.vaos);
                    resources.vaos[id] = vao;
                    ids[i] = id;
                }
            },
            glBindVertexArray: (id) => {
                if (gl.bindVertexArray) {
                    gl.bindVertexArray(resources.vaos[id]);
                }
            },

            // Input handling setup
            setup_canvas_size: (highDpi) => {
                const dpi = highDpi ? (window.devicePixelRatio || 1) : 1;
                canvas.width = canvas.clientWidth * dpi;
                canvas.height = canvas.clientHeight * dpi;
            },

            run_animation_loop: () => {
                setupInputHandlers();
                requestAnimationFrame(gameLoop);
            },

            // Clipboard (stubs)
            sapp_set_clipboard: () => {},

            // Misc stubs for functions we might not need yet
            glGetIntegerv: () => {},
            glPixelStorei: () => {},
            glFlush: () => {},
            glFinish: () => {},
            glScissor: () => {},
            glColorMask: () => {},
            glClearStencil: () => {},
            glBlendEquationSeparate: () => {},
            glBlendFuncSeparate: () => {},
            glCullFace: () => {},
            glFrontFace: () => {},
            glDepthMask: () => {},
            glStencilMask: () => {},
            glStencilFunc: () => {},
            glStencilOp: () => {},
            glReadPixels: () => {},
            glGenFramebuffers: () => {},
            glBindFramebuffer: () => {},
            glFramebufferTexture2D: () => {},
            glGenRenderbuffers: () => {},
            glBindRenderbuffer: () => {},
            glRenderbufferStorage: () => {},
            glFramebufferRenderbuffer: () => {},
            glCheckFramebufferStatus: () => 0x8CD5, // GL_FRAMEBUFFER_COMPLETE
            glDeleteTextures: () => {},
            glDeleteBuffers: () => {},
            glDeleteFramebuffers: () => {},
            glDeleteRenderbuffers: () => {},
            glDeleteProgram: () => {},
            glGetString: () => 0,
            glGetProgramInfoLog: () => {},
            set_emscripten_shader_hack: () => {},
        }
    };

    // Input handling
    function setupInputHandlers() {
        // Mouse movement
        canvas.addEventListener('mousemove', (e) => {
            const rect = canvas.getBoundingClientRect();
            const x = (e.clientX - rect.left) * (canvas.width / rect.width);
            const y = (e.clientY - rect.top) * (canvas.height / rect.height);
            if (wasmExports.mouse_move) {
                wasmExports.mouse_move(Math.floor(x), Math.floor(y));
            }
        });

        // Mouse buttons
        canvas.addEventListener('mousedown', (e) => {
            const rect = canvas.getBoundingClientRect();
            const x = (e.clientX - rect.left) * (canvas.width / rect.width);
            const y = (e.clientY - rect.top) * (canvas.height / rect.height);
            if (wasmExports.mouse_down) {
                wasmExports.mouse_down(x, y, e.button);
            }
        });

        canvas.addEventListener('mouseup', (e) => {
            const rect = canvas.getBoundingClientRect();
            const x = (e.clientX - rect.left) * (canvas.width / rect.width);
            const y = (e.clientY - rect.top) * (canvas.height / rect.height);
            if (wasmExports.mouse_up) {
                wasmExports.mouse_up(x, y, e.button);
            }
        });

        // Keyboard
        const keyMap = {
            'Space': 32, 'KeyA': 65, 'KeyB': 66, 'KeyC': 67, 'KeyD': 68,
            'KeyE': 69, 'KeyF': 70, 'KeyG': 71, 'KeyH': 72, 'KeyI': 73,
            'KeyJ': 74, 'KeyK': 75, 'KeyL': 76, 'KeyM': 77, 'KeyN': 78,
            'KeyO': 79, 'KeyP': 80, 'KeyQ': 81, 'KeyR': 82, 'KeyS': 83,
            'KeyT': 84, 'KeyU': 85, 'KeyV': 86, 'KeyW': 87, 'KeyX': 88,
            'KeyY': 89, 'KeyZ': 90, 'Escape': 256, 'Enter': 257, 'Tab': 258,
            'ArrowRight': 262, 'ArrowLeft': 263, 'ArrowDown': 264, 'ArrowUp': 265,
            'Shift': 340, 'Control': 341, 'Alt': 342
        };

        canvas.addEventListener('keydown', (e) => {
            const keyCode = keyMap[e.code] || 0;
            if (keyCode && wasmExports.key_down) {
                wasmExports.key_down(keyCode, 0, e.repeat);
                e.preventDefault();
            }
        });

        canvas.addEventListener('keyup', (e) => {
            const keyCode = keyMap[e.code] || 0;
            if (keyCode && wasmExports.key_up) {
                wasmExports.key_up(keyCode, 0);
                e.preventDefault();
            }
        });

        // Focus canvas for keyboard input
        canvas.setAttribute('tabindex', '0');
        canvas.focus();
    }

    // Game loop
    function gameLoop() {
        if (wasmExports && wasmExports.frame) {
            wasmExports.frame();
        }
        animationId = requestAnimationFrame(gameLoop);
    }

    // Main loader function
    window.load = async function(wasmPath) {
        console.log('Loading WASM from:', wasmPath);

        try {
            // Initialize WebGL first
            if (!initWebGL()) {
                throw new Error('Failed to initialize WebGL');
            }

            // Fetch and compile WASM
            const response = await fetch(wasmPath);
            if (!response.ok) {
                throw new Error(`Failed to fetch WASM: ${response.statusText}`);
            }

            const wasmBytes = await response.arrayBuffer();
            const wasmModule = await WebAssembly.compile(wasmBytes);

            // Instantiate with our import object
            const instance = await WebAssembly.instantiate(wasmModule, importObject);

            // Store exports and memory
            wasmExports = instance.exports;
            wasmMemory = instance.exports.memory;

            console.log('WASM loaded successfully');

            // Call the main function if it exists
            if (wasmExports.main) {
                wasmExports.main();
            }

        } catch (error) {
            console.error('Failed to load WASM:', error);
            alert('Failed to load the game. Check console for details.');
        }
    };

})();
