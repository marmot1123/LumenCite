// dist/ に manifest.json と public/ の静的ファイルをコピーする（ビルドの後段）。
import { cpSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(root, "dist");

mkdirSync(dist, { recursive: true });
cpSync(join(root, "manifest.json"), join(dist, "manifest.json"));
cpSync(join(root, "public"), dist, { recursive: true });

console.log("copied static files to dist/");
