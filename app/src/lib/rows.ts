// Decoding the row windows the backend sends: a little-endian u32 row
// count, then one length-prefixed byte string per cell (0xFB is NULL).

const decoder = new TextDecoder();

/** Decodes up to `cells` cells from the start of a window into text (null for NULL). */
export function decodeCells(u8: Uint8Array, cells: number): { rows: number; values: (string | null)[] } {
  const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength);
  const rows = dv.getUint32(0, true);
  const values: (string | null)[] = [];
  let pos = 4;
  while (values.length < cells && pos < u8.length) {
    const b = u8[pos];
    let start: number;
    let len: number;
    if (b === 0xfb) {
      values.push(null);
      pos += 1;
      continue;
    } else if (b < 251) {
      start = pos + 1;
      len = b;
    } else if (b === 0xfc) {
      start = pos + 3;
      len = dv.getUint16(pos + 1, true);
    } else if (b === 0xfd) {
      start = pos + 4;
      len = u8[pos + 1] | (u8[pos + 2] << 8) | (u8[pos + 3] << 16);
    } else {
      start = pos + 9;
      len = dv.getUint32(pos + 1, true);
    }
    values.push(decoder.decode(u8.subarray(start, start + len)));
    pos = start + len;
  }
  return { rows, values };
}
