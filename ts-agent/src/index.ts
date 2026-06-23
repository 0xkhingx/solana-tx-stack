import "dotenv/config";
import fs from "node:fs";
import path from "node:path";
import { Agent } from "./agent.js";
import { RetryTrigger } from "./retry-trigger.js";

const repoRoot = path.resolve(process.cwd(), "..");
const apiKey = process.env.ANTHROPIC_API_KEY;

if (!apiKey) {
  throw new Error("ANTHROPIC_API_KEY is required");
}

console.log("[AGENT] Starting TS agent watcher loop");
new RetryTrigger(repoRoot).start();

// Polling loop watches the lifecycle log for newly failed bundles.
const agent = new Agent(repoRoot, apiKey);
const seen = new Set<string>();

setInterval(() => {
  const file = path.join(repoRoot, "logs", "lifecycle.jsonl");
  try {
    const text = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
    const lines = text.trim().split(/\r?\n/).filter(Boolean);
    for (const line of lines) {
      const entry = JSON.parse(line) as { bundle_id: string; failure_reason?: unknown; retry_count: number };
      if (!seen.has(entry.bundle_id) && entry.failure_reason != null && entry.retry_count === 0) {
        seen.add(entry.bundle_id);
        void agent.processFailure(entry as never);
      }
    }
  } catch (error) {
    console.error("[AGENT] watcher error:", error);
  }
}, 500);
