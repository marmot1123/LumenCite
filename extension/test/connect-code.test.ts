import { describe, expect, it } from "vitest";
import { parseConnectCode } from "../src/connect-code.js";

function encode(payload: unknown): string {
  const b64 = btoa(JSON.stringify(payload));
  return "lc1." + b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

describe("parseConnectCode", () => {
  it("parses a valid code (whitespace-tolerant)", () => {
    const code = encode({ v: 1, port: 3917, token: "a".repeat(48) });
    expect(parseConnectCode(`  ${code}\n`)).toEqual({ port: 3917, token: "a".repeat(48) });
  });

  it("rejects wrong prefix, garbage base64, and bad JSON", () => {
    expect(parseConnectCode("lc2.abc")).toBeNull();
    expect(parseConnectCode("lc1.")).toBeNull();
    expect(parseConnectCode("lc1.!!!!")).toBeNull();
    expect(parseConnectCode("lc1." + btoa("not json"))).toBeNull();
    expect(parseConnectCode("")).toBeNull();
  });

  it("rejects missing or invalid fields", () => {
    expect(parseConnectCode(encode({ v: 2, port: 3917, token: "t" }))).toBeNull();
    expect(parseConnectCode(encode({ v: 1, token: "t" }))).toBeNull();
    expect(parseConnectCode(encode({ v: 1, port: 0, token: "t" }))).toBeNull();
    expect(parseConnectCode(encode({ v: 1, port: 70000, token: "t" }))).toBeNull();
    expect(parseConnectCode(encode({ v: 1, port: 3917, token: "" }))).toBeNull();
    expect(parseConnectCode(encode({ v: 1, port: "3917", token: "t" }))).toBeNull();
  });
});
