/* @ts-self-types="./tobari.d.ts" */

/**
 * Compute altitude profile for all 3 atmosphere models.
 *
 * Returns flat `[exp_0, hp_0, msis_0, exp_1, hp_1, msis_1, ...]` (length = N×3).
 * @param {Float64Array} altitudes
 * @param {number} lat_deg
 * @param {number} lon_deg
 * @param {number} epoch_jd
 * @param {number} f107
 * @param {number} ap
 * @returns {Float64Array}
 */
export function atmosphere_altitude_profile(altitudes, lat_deg, lon_deg, epoch_jd, f107, ap) {
    const ptr0 = passArrayF64ToWasm0(altitudes, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.atmosphere_altitude_profile(ptr0, len0, lat_deg, lon_deg, epoch_jd, f107, ap);
    var v2 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v2;
}

/**
 * Compute lat/lon density map for a chosen atmosphere model.
 *
 * `model`: `"exponential"`, `"harris-priester"`, or `"nrlmsise00"`.
 * Returns flat row-major `[rho_0, rho_1, ...]` (length = n_lat × n_lon).
 * Latitude ranges from -90 to +90, longitude from -180 to +180.
 * @param {string} model
 * @param {number} altitude_km
 * @param {number} epoch_jd
 * @param {number} n_lat
 * @param {number} n_lon
 * @param {number} f107
 * @param {number} ap
 * @returns {Float64Array}
 */
export function atmosphere_latlon_map(model, altitude_km, epoch_jd, n_lat, n_lon, f107, ap) {
    const ptr0 = passStringToWasm0(model, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.atmosphere_latlon_map(ptr0, len0, altitude_km, epoch_jd, n_lat, n_lon, f107, ap);
    var v2 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v2;
}

/**
 * Compute 3D atmospheric density volume as Float32.
 *
 * Layout: alt-major `index = iAlt * nLat * nLon + iLat * nLon + iLon`
 * Returns `[rho_0, rho_1, ...]` (length = n_alt × n_lat × n_lon).
 * Also returns `[min, max]` appended at the end (total length = n_alt*n_lat*n_lon + 2).
 * @param {string} model
 * @param {number} alt_min_km
 * @param {number} alt_max_km
 * @param {number} n_alt
 * @param {number} epoch_jd
 * @param {number} n_lat
 * @param {number} n_lon
 * @param {number} f107
 * @param {number} ap
 * @returns {Float32Array}
 */
export function atmosphere_volume(model, alt_min_km, alt_max_km, n_alt, epoch_jd, n_lat, n_lon, f107, ap) {
    const ptr0 = passStringToWasm0(model, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.atmosphere_volume(ptr0, len0, alt_min_km, alt_max_km, n_alt, epoch_jd, n_lat, n_lon, f107, ap);
    var v2 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v2;
}

/**
 * Tilted dipole field at a geodetic point.
 *
 * Returns `[B_north, B_east, B_down, |B|, inclination_deg, declination_deg]` in nT.
 * @param {number} lat_deg
 * @param {number} lon_deg
 * @param {number} altitude_km
 * @param {number} epoch_jd
 * @returns {Float64Array}
 */
export function dipole_field_at(lat_deg, lon_deg, altitude_km, epoch_jd) {
    const ret = wasm.dipole_field_at(lat_deg, lon_deg, altitude_km, epoch_jd);
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Exponential atmosphere density [kg/m³] at the given altitude.
 * @param {number} altitude_km
 * @returns {number}
 */
export function exponential_density(altitude_km) {
    const ret = wasm.exponential_density(altitude_km);
    return ret;
}

/**
 * Harris-Priester density [kg/m³] at a geodetic point and epoch.
 *
 * `epoch_jd`: Julian Date of the epoch.
 * @param {number} lat_deg
 * @param {number} lon_deg
 * @param {number} altitude_km
 * @param {number} epoch_jd
 * @returns {number}
 */
export function harris_priester_density(lat_deg, lon_deg, altitude_km, epoch_jd) {
    const ret = wasm.harris_priester_density(lat_deg, lon_deg, altitude_km, epoch_jd);
    return ret;
}

/**
 * IGRF-14 field at a geodetic point.
 *
 * Returns `[B_north, B_east, B_down, |B|, inclination_deg, declination_deg]` in nT.
 * @param {number} lat_deg
 * @param {number} lon_deg
 * @param {number} altitude_km
 * @param {number} epoch_jd
 * @returns {Float64Array}
 */
export function igrf_field_at(lat_deg, lon_deg, altitude_km, epoch_jd) {
    const ret = wasm.igrf_field_at(lat_deg, lon_deg, altitude_km, epoch_jd);
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Compute lat/lon magnetic field map.
 *
 * `model`: `"igrf"` or `"dipole"`.
 * `component`: `"total"`, `"inclination"`, `"declination"`, `"north"`, `"east"`, `"down"`.
 * Returns flat row-major values (length = n_lat × n_lon).
 * Values in nT for field components, degrees for angles.
 * @param {string} model
 * @param {string} component
 * @param {number} altitude_km
 * @param {number} epoch_jd
 * @param {number} n_lat
 * @param {number} n_lon
 * @returns {Float64Array}
 */
export function magnetic_field_latlon_map(model, component, altitude_km, epoch_jd, n_lat, n_lon) {
    const ptr0 = passStringToWasm0(model, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(component, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.magnetic_field_latlon_map(ptr0, len0, ptr1, len1, altitude_km, epoch_jd, n_lat, n_lon);
    var v3 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v3;
}

/**
 * Integrate magnetic field lines from seed points using RK4.
 *
 * `seed_lats`, `seed_lons`: geodetic seed points (degrees).
 * `seed_alt_km`: starting altitude for all seeds.
 * `model`: `"igrf"` or `"dipole"`.
 * `max_steps`: max integration steps per line.
 * `step_km`: step size in km.
 *
 * Returns flat `[n_lines, n_pts_0, x0,y0,z0, x1,y1,z1, ..., n_pts_1, ...]`
 * where coordinates are in Earth radii (6371 km).
 * @param {Float64Array} seed_lats
 * @param {Float64Array} seed_lons
 * @param {number} seed_alt_km
 * @param {number} epoch_jd
 * @param {string} model
 * @param {number} max_steps
 * @param {number} step_km
 * @returns {Float32Array}
 */
export function magnetic_field_lines(seed_lats, seed_lons, seed_alt_km, epoch_jd, model, max_steps, step_km) {
    const ptr0 = passArrayF64ToWasm0(seed_lats, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArrayF64ToWasm0(seed_lons, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    const ptr2 = passStringToWasm0(model, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len2 = WASM_VECTOR_LEN;
    const ret = wasm.magnetic_field_lines(ptr0, len0, ptr1, len1, seed_alt_km, epoch_jd, ptr2, len2, max_steps, step_km);
    var v4 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v4;
}

/**
 * NRLMSISE-00 density [kg/m³] at a geodetic point with constant space weather.
 *
 * `f107`: F10.7 solar radio flux [SFU].
 * `ap`: daily Ap geomagnetic index.
 * @param {number} lat_deg
 * @param {number} lon_deg
 * @param {number} altitude_km
 * @param {number} epoch_jd
 * @param {number} f107
 * @param {number} ap
 * @returns {number}
 */
export function nrlmsise00_density(lat_deg, lon_deg, altitude_km, epoch_jd, f107, ap) {
    const ret = wasm.nrlmsise00_density(lat_deg, lon_deg, altitude_km, epoch_jd, f107, ap);
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
        "./tobari_bg.js": import0,
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

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function passArrayF64ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 8, 8) >>> 0;
    getFloat64ArrayMemory0().set(arg, ptr / 8);
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
        module_or_path = new URL('tobari_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
