import { rt } from "../core.js";

export const matchUrl = /^(https?:)?\/\//;

export function dirname(path) {
    return rt.call("dirname", path);
}

export function resolve(url, currentDirectory, cwd) {
    return rt.call("resolve", url, currentDirectory, cwd);
}
