// Mock workspace data for the prototype. Titles and branches come from real Clinch sessions so
// the layout is judged against realistic lengths, not lorem ipsum.

export type Kind = "claude" | "codex" | "shell";
export type Status = "working" | "needs" | "done" | "idle" | "limited";
export type Transcript =
  | "shopify"
  | "answer"
  | "coordinator"
  | "permission"
  | "limited"
  | "shell"
  | "shellFailed"
  | "fresh";

export interface Tab {
  id: string;
  title: string;
  branch: string;
  kind: Kind;
  status: Status;
  child?: boolean;
  pinned?: boolean;
  coordinator?: boolean;
  /** Shown while Clinch reattaches the session after a restart. */
  restoring?: boolean;
  transcript: Transcript;
  cwd: string;
  pr?: number;
  diff?: [number, number];
}

export type SectionColor = "lime" | "coral" | "blue" | "amber" | "violet" | "teal" | "pink" | "gray";

export interface Section {
  id: string;
  name: string;
  color: SectionColor;
  open: boolean;
  tabs: Tab[];
  /** Tabs not drawn in the prototype but counted, so collapsed sections show real totals. */
  hiddenCount?: number;
  hiddenNeeds?: number;
}

export interface Project {
  id: string;
  name: string;
  waiting: number;
  coordinator: Tab;
  ungrouped: Tab[];
  sections: Section[];
  tasks: string[];
  bookmarks: string[];
}

const MM = "~/.clinch/worktrees/magister-marketing";
const CT = "~/.clinch/worktrees/clinch-terminal";

let n = 0;
const t = (
  title: string,
  branch: string,
  kind: Kind,
  status: Status,
  transcript: Transcript,
  extra: Partial<Tab> = {},
): Tab => ({
  id: `t${++n}`,
  title,
  branch,
  kind,
  status,
  transcript,
  cwd: extra.cwd ?? `${MM}/${branch.replace(/\//g, "-")}`,
  ...extra,
});

const coordinator = (project: string, status: Status, cwd: string): Tab =>
  t("Coordinator", project, "claude", status, "coordinator", { pinned: true, coordinator: true, cwd });

export const PROJECTS: Project[] = [
  {
    id: "magister",
    name: "magister-marketing",
    waiting: 14,
    coordinator: coordinator("magister-marketing", "done", "~/projects/magister-marketing"),
    ungrouped: [],
    sections: [
      {
        id: "p0",
        name: "P0",
        color: "lime",
        open: true,
        hiddenNeeds: 0,
        tabs: [
          t("Above-the-fold landing page redesign", "bajada-plateau", "claude", "working", "shopify"),
          t("Magister gateway HTTP 5xx spikes", "coord/2256", "claude", "needs", "permission"),
          t("Clinch Codex session continuation", "feat/strapi-media-upload", "claude", "done", "answer"),
          t("Codex clinch session", "dawn-sotol", "claude", "idle", "answer"),
          t("Audit content memory behavior", "tarantula-prairie-dog", "codex", "done", "answer"),
          t("Project warning dismissal options", "fix/connection-attempt-notices", "claude", "needs", "permission"),
          t("Refine agent box spacing", "ocotillo-sotol", "codex", "idle", "answer"),
          t("Check computer use access", "gap-smoke", "codex", "done", "answer", { child: true }),
          t("Check Shopify OAuth submission", "codex/shopify-review-readiness", "codex", "working", "shopify"),
          t("Codex session continuation", "gecko-thermal", "claude", "working", "shopify", { pr: 2170, diff: [214, 37] }),
          t("Ultrareview PR 1905 and openclaw fork", "cite-gorge/claude-run", "claude", "limited", "limited"),
          t("User complaint: upgrade checkout error", "fix/connect-agent-reupgrade", "claude", "done", "answer"),
          t("User complaint screenshot", "fix/connect-agent-reupgrade", "claude", "idle", "answer", { child: true }),
          t("dev server", "main", "shell", "working", "shell", { cwd: "~/projects/magister-marketing/webapp" }),
        ],
        hiddenCount: 4,
      },
      {
        id: "p1",
        name: "P1",
        color: "coral",
        open: true,
        tabs: [
          t("AI visibility: Perplexity + Grok (#2154)", "f35f69e55", "claude", "needs", "permission"),
          t("Harness bench S1: done, lanes off", "plan/pi-core-benchmark", "claude", "done", "answer"),
          t("Onboarding walk (onepr) + own-agent loop research", "integrate/one-pr", "claude", "idle", "answer", { child: true }),
          t("Onboarding walk (onepr) + Firecrawl people search", "integrate/one-pr", "claude", "limited", "limited", { child: true }),
        ],
      },
      {
        id: "todo",
        name: "TODOS",
        color: "gray",
        open: false,
        tabs: [
          t("Retire max-runs flag", "fix/max-runs-retirement", "claude", "idle", "answer"),
          t("Pricing page copy pass", "copy/pricing", "claude", "idle", "answer"),
        ],
        hiddenCount: 3,
      },
      {
        id: "prog",
        name: "In progress",
        color: "blue",
        open: true,
        tabs: [
          t("Sentry errors from Magister alerts", "fix/sentry-triage-2026-10-01", "claude", "needs", "permission"),
          t("Facebook integration OAuth connection", "6ca5b8710", "claude", "done", "answer", { pr: 2163, diff: [88, 12] }),
          t("Add simple reminders without cron jobs", "fix/max-runs-retirement", "claude", "idle", "answer"),
          t("Review this", "codex/account-free-public-audits", "codex", "working", "shopify"),
          t("cargo nextest", "main", "shell", "idle", "shellFailed", { cwd: "~/projects/magister-marketing/gateway" }),
        ],
      },
      {
        id: "follow",
        name: "Followup on these chats",
        color: "amber",
        open: false,
        tabs: [
          t("TypeSafe AI jev model integration", "dawn-flint", "claude", "done", "answer"),
          t("Turn this into an isolated open source project", "codex/mcp-ai-visibility-contracts", "codex", "needs", "permission"),
          t("Add graph rendering capability to chat interface", "candelilla-canyon", "claude", "idle", "answer"),
        ],
        hiddenCount: 7,
        hiddenNeeds: 2,
      },
    ],
    tasks: ["Why did bruno have 700 credits?", "Reply to the Shopify review email", "Check Sentry after deploy"],
    bookmarks: ["Shopify OAuth submission notes"],
  },
  {
    id: "clinch",
    name: "clinch-terminal",
    waiting: 0,
    coordinator: coordinator("clinch-terminal", "idle", "~/projects/clinch-terminal"),
    ungrouped: [
      t("Codex session handoff in Clinch", "jojoba-quail", "claude", "idle", "answer", { cwd: `${CT}/jojoba-quail` }),
      t("Codex session handoff in Clinch", "jojoba-quail", "claude", "done", "answer", { child: true, cwd: `${CT}/jojoba-quail` }),
      t("zsh", "main", "shell", "idle", "shell", { cwd: "~/projects/clinch-terminal" }),
      t("Agent needs-input blink", "basin-quail", "claude", "idle", "fresh", { restoring: true, cwd: `${CT}/basin-quail` }),
    ],
    sections: [],
    tasks: [],
    bookmarks: [],
  },
  {
    id: "gradient",
    name: "gradient",
    waiting: 1,
    coordinator: coordinator("gradient", "needs", "~/projects/gradient"),
    ungrouped: [t("Optimize setup report", "main", "claude", "working", "shopify", { cwd: "~/projects/gradient" })],
    sections: [],
    tasks: [],
    bookmarks: [],
  },
  {
    id: "rakazo",
    name: "rakazo",
    waiting: 0,
    coordinator: coordinator("rakazo", "idle", "~/projects/rakazo"),
    ungrouped: [],
    sections: [],
    tasks: [],
    bookmarks: [],
  },
];

export function allTabs(project: Project): Tab[] {
  return [project.coordinator, ...project.ungrouped, ...project.sections.flatMap((s) => s.tabs)];
}

export const LAST_PROMPT: Record<Transcript, string> = {
  shopify: "And review all the Shopify docs and guidelines too when we’re doing our testing.",
  answer: "Could we fork Ghostty instead, or drastically change what Clinch looks like?",
  coordinator: "Keep an eye on the gateway deploys and tell me before anything ships.",
  permission: "Find out why the gateway throws 5xx at 14:00 and fix it on staging first.",
  limited: "Run ultrareview on PR 1905 and compare against the openclaw fork.",
  shell: "",
  shellFailed: "",
  fresh: "",
};

export const STATUS_WORD: Record<Status, string> = {
  working: "working",
  needs: "needs you",
  done: "done, unread",
  idle: "idle",
  limited: "rate limited",
};

export const PROVIDER_NAME: Record<Kind, string> = {
  claude: "Claude Code",
  codex: "Codex",
  shell: "Terminal",
};
