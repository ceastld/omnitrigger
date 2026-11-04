/** @param {unknown} value */
export function cleanText(value) {
    return String(value ?? '').trim();
}
