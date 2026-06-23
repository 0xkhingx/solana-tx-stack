import fs from "node:fs";
import path from "node:path";

type Decision = {
  action: "retry" | "abort";
  refresh_blockhash: boolean;
  new_tip_lamports: number;
  reasoning: string;
};

export class RetryTrigger {
  constructor(private readonly repoRoot: string) {}

  start(): void {
    const decisionsDir = path.join(this.repoRoot, "decisions");
    fs.mkdirSync(decisionsDir, { recursive: true });
    fs.watch(decisionsDir, (_eventType, filename) => {
      if (!filename?.endsWith(".json")) return;
      const file = path.join(decisionsDir, filename);
      if (!fs.existsSync(file)) return;
      const decision = JSON.parse(fs.readFileSync(file, "utf8")) as Decision;
      if (decision.action === "retry") {
        const retryQueue = path.join(this.repoRoot, "retry-queue.json");
        fs.writeFileSync(
          retryQueue,
          JSON.stringify(
            {
              bundle_id: filename.replace(/\.json$/u, ""),
              new_tip_lamports: decision.new_tip_lamports,
              refresh_blockhash: decision.refresh_blockhash,
            },
            null,
            2,
          ),
        );
      } else {
        console.log(`[AGENT] Abort requested: ${decision.reasoning}`);
      }
    });
  }
}
