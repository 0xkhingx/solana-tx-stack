import fs from "node:fs";
import path from "node:path";

export type TipStats = {
  p50: number;
  p75: number;
  p90: number;
  values: number[];
};

function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p));
  return sorted[idx];
}

export function readTipStats(repoRoot: string): TipStats {
  const tipFile = path.join(repoRoot, "logs", "tips.json");
  if (!fs.existsSync(tipFile)) {
    return { p50: 0, p75: 0, p90: 0, values: [] };
  }
  const raw = JSON.parse(fs.readFileSync(tipFile, "utf8")) as { values?: number[] };
  const values = Array.isArray(raw.values) ? raw.values.filter((n) => Number.isFinite(n)) : [];
  return {
    values,
    p50: percentile(values, 0.5),
    p75: percentile(values, 0.75),
    p90: percentile(values, 0.9),
  };
}
