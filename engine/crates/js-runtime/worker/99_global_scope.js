// Global scope registration for host_v8_worker APIs (api-system feature gate).

import * as workerApi from 'ext:host_v8_worker/01_worker.js';

import { primordials, core } from "ext:core/mod.js";
const { ObjectDefineProperties } = primordials;

ObjectDefineProperties(globalThis, {
    // Worker
    createWorker: core.propNonEnumerable(workerApi.createWorker),
});
