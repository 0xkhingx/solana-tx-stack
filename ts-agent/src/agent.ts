import fs from "node:fs";
import path from "node:path";
import Anthropic from "@anthropic-ai/sdk";
import { readTipStats } from "./tip-reader.js";

type LifecycleEntry = {
  bundle_id: string;
  tip_lamports: number;
  submitted_at_slot?: number | null;
  failure_reason?: unknown;
  retry_count: number;
};

type AgentDecision = {
  action: "retry" | "abort";
  refresh_blockhash: boolean;
  new_tip_lamports: number;
  reasoning: string;
};

export class Agent {
  private readonly anthropic: Anthropic;
  private readonly repoRoot: string;

  constructor(repoRoot: string, apiKey: string) {
    this.repoRoot = repoRoot;
    this.anthropic = new Anthropic({ apiKey });
  }

  async processFailure(entry: LifecycleEntry): Promise<void> {
    const logPath = path.join(this.repoRoot, "logs", "lifecycle.jsonl");
    const tipStats = readTipStats(this.repoRoot);
    const recent = this.readRecentOutcomes(logPath, 3);
    const prompt = [
      `Failure reason: ${JSON.stringify(entry.failure_reason)}`,
      `Tip amount used: ${entry.tip_lamports}`,
      `Submission slot: ${entry.submitted_at_slot ?? "unknown"}`,
      `Tip stats: ${JSON.stringify({ p50: tipStats.p50, p75: tipStats.p75, p90: tipStats.p90 })}`,
      `Recent outcomes: ${JSON.stringify(recent)}`,
    ].join("\n");

    const response = await this.anthropic.messages.create({
      model: "claude-sonnet-4-6",
      max_tokens: 512,
      system:
        'Respond ONLY with valid JSON: {"action":"retry"|"abort","refresh_blockhash":boolean,"new_tip_lamports":number,"reasoning":"string explaining the decision"}',
      messages: [{ role: "user", content: prompt }],
    });

    const text = response.content
      .map((part) => ("text" in part ? part.text : ""))
      .join("")
      .trim();

    let decision: AgentDecision;
    try {
      decision = JSON.parse(text) as AgentDecision;
    } catch (error) {
      console.error("[AGENT] Failed to parse Claude JSON, defaulting to abort:", error);
      decision = {
        action: "abort",
        refresh_blockhash: false,
        new_tip_lamports: 0,
        reasoning: "Defaulted to abort because Claude response was invalid JSON.",
      };
    }

    const decisionsDir = path.join(this.repoRoot, "decisions");
    fs.mkdirSync(decisionsDir, { recursive: true });
    fs.writeFileSync(path.join(decisionsDir, `${entry.bundle_id}.json`), JSON.stringify(decision, null, 2));
    console.log(`[AGENT] ${decision.reasoning}`);
  }

  private readRecentOutcomes(logPath: string, limit: number): unknown[] {
    if (!fs.existsSync(logPath)) return [];
    const lines = fs.readFileSync(logPath, "utf8").trim().split(/\r?\n/).filter(Boolean);
    return lines.slice(-limit).map((line) => {
      try {
        return JSON.parse(line);
      } catch {
        return null;
      }
    });
  }
}
