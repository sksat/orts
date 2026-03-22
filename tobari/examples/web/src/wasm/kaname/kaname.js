/* @ts-self-types="./kaname.d.ts" */

/**
 * Compute the Earth Rotation Angle (GMST) in radians.
 *
 * `epoch_jd`: Julian Date of the simulation epoch
 * `t`: elapsed simulation time in seconds
 * @param {number} epoch_jd
 * @param {number} t
 * @returns {number}
 */
export function earth_rotation_angle(epoch_jd, t) {
    const ret = wasm.earth_rotation_angle(epoch_jd, t);
    return ret;
}

/**
 * Single-point ECI→ECEF transform.
 *
 * Returns flat ECEF `[ex, ey, ez]` (3 floats, km).
 * @param {number} x
 * @param {number} y
 * @param {number} z
 * @param {number} epoch_jd
 * @param {number} t
 * @returns {Float32Array}
 */
export function eci_to_ecef(x, y, z, epoch_jd, t) {
    const ret = wasm.eci_to_ecef(x, y, z, epoch_jd, t);
    var v1 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * Batch ECI→ECEF transform with per-point time.
 *
 * `positions`: flat `[x0,y0,z0, x1,y1,z1, ...]` (length = N×3, km)
 * `times`: `[t0, t1, ...]` (length = N, simulation elapsed seconds)
 * `epoch_jd`: Julian Date of the simulation epoch
 *
 * Returns flat ECEF `[ex0,ey0,ez0, ...]` (length = N×3, km).
 *
 * For each point, computes ERA from `epoch_jd + t` and applies the
 * Z-axis rotation (ECI→ECEF).
 * @param {Float32Array} positions
 * @param {Float32Array} times
 * @param {number} epoch_jd
 * @returns {Float32Array}
 */
export function eci_to_ecef_batch(positions, times, epoch_jd) {
    const ptr0 = passArrayF32ToWasm0(positions, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArrayF32ToWasm0(times, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.eci_to_ecef_batch(ptr0, len0, ptr1, len1, epoch_jd);
    var v3 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v3;
}

/**
 * Geodetic (lat_deg, lon_deg, altitude_km) → ECEF [km].
 *
 * Returns `[x, y, z]` (3 floats, km).
 * @param {number} lat_deg
 * @param {number} lon_deg
 * @param {number} altitude_km
 * @returns {Float64Array}
 */
export function geodetic_to_ecef(lat_deg, lon_deg, altitude_km) {
    const ret = wasm.geodetic_to_ecef(lat_deg, lon_deg, altitude_km);
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Geodetic (lat_deg, lon_deg, altitude_km) → ECI [km] at given epoch.
 *
 * Returns `[x, y, z]` (3 floats, km).
 * @param {number} lat_deg
 * @param {number} lon_deg
 * @param {number} altitude_km
 * @param {number} epoch_jd
 * @returns {Float64Array}
 */
export function geodetic_to_eci(lat_deg, lon_deg, altitude_km, epoch_jd) {
    const ret = wasm.geodetic_to_eci(lat_deg, lon_deg, altitude_km, epoch_jd);
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Convert Julian Date + elapsed sim time to a UTC date/time string.
 *
 * Returns ISO 8601 string like "2024-03-20T12:00:00Z".
 * @param {number} epoch_jd
 * @param {number} t
 * @returns {string}
 */
export function jd_to_utc_string(epoch_jd, t) {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.jd_to_utc_string(epoch_jd, t);
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Approximate sun direction (unit vector) in ECI frame.
 *
 * Returns `[x, y, z]` (3 floats).
 * @param {number} epoch_jd
 * @param {number} t
 * @returns {Float32Array}
 */
export function sun_direction_eci(epoch_jd, t) {
    const ret = wasm.sun_direction_eci(epoch_jd, t);
    var v1 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * Sun direction (unit vector) as seen from a given central body, in J2000 equatorial frame.
 *
 * Returns `[x, y, z]` (3 floats).
 * `body`: body identifier string (e.g., "earth", "mars")
 * `epoch_jd`: Julian Date of the simulation epoch
 * `t`: elapsed simulation time in seconds
 * @param {string} body
 * @param {number} epoch_jd
 * @param {number} t
 * @returns {Float32Array}
 */
export function sun_direction_from_body(body, epoch_jd, t) {
    const ptr0 = passStringToWasm0(body, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.sun_direction_from_body(ptr0, len0, epoch_jd, t);
    var v2 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v2;
}

/**
 * Sun distance [km] from a given central body.
 *
 * `body`: body identifier string (e.g., "earth", "mars")
 * `epoch_jd`: Julian Date of the simulation epoch
 * `t`: elapsed simulation time in seconds
 * @param {string} body
 * @param {number} epoch_jd
 * @param {number} t
 * @returns {number}
 */
export function sun_distance_from_body(body, epoch_jd, t) {
    const ptr0 = passStringToWasm0(body, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.sun_distance_from_body(ptr0, len0, epoch_jd, t);
    return ret;
}

function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./kaname_bg.js": import0,
    };
}

function getArrayF32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayF64FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat64ArrayMemory0().subarray(ptr / 8, ptr / 8 + len);
}

let cachedFloat32ArrayMemory0 = null;
function getFloat32ArrayMemory0() {
    if (cachedFloat32ArrayMemory0 === null || cachedFloat32ArrayMemory0.byteLength === 0) {
        cachedFloat32ArrayMemory0 = new Float32Array(wasm.memory.buffer);
    }
    return cachedFloat32ArrayMemory0;
}

let cachedFloat64ArrayMemory0 = null;
function getFloat64ArrayMemory0() {
    if (cachedFloat64ArrayMemory0 === null || cachedFloat64ArrayMemory0.byteLength === 0) {
        cachedFloat64ArrayMemory0 = new Float64Array(wasm.memory.buffer);
    }
    return cachedFloat64ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return decodeText(ptr, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function passArrayF32ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 4, 4) >>> 0;
    getFloat32ArrayMemory0().set(arg, ptr / 4);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasm;
function __wbg_finalize_init(instance, module) {
    wasm = instance.exports;
    wasmModule = module;
    cachedFloat32ArrayMemory0 = null;
    cachedFloat64ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('kaname_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
