// Git provider catalog: labels, connection form hints, and token guides.
// Forgejo speaks the Gitea API — same endpoints, different headers.
export const GIT_PROVIDERS = ["github", "gitea", "forgejo", "gitlab", "bitbucket"] as const;
export type GitProvider = (typeof GIT_PROVIDERS)[number];

export function providerLabel(p: string): string {
  if (p === "github") return "GitHub";
  if (p === "forgejo") return "Forgejo";
  return p[0].toUpperCase() + p.slice(1);
}

export interface ProviderGuide {
  /** Placeholder for the instance URL input (absent = no URL input). */
  basePlaceholder?: string;
  tokenPlaceholder: string;
  /** Show the workspace-slug input (Bitbucket). */
  workspace?: boolean;
  steps: string[];
  docsUrl: string;
  docsLabel: string;
}

export const PROVIDER_GUIDES: Record<Exclude<GitProvider, "github">, ProviderGuide> = {
  gitea: {
    basePlaceholder: "https://git.example.com",
    tokenPlaceholder: "access token",
    steps: [
      "On your Gitea instance: avatar → Settings → Applications → Generate New Token.",
      "Grant repository read access (plus user read), then paste the token here.",
      "Add the webhook URL from your project afterwards for push deploys.",
    ],
    docsUrl: "https://docs.gitea.com/",
    docsLabel: "Gitea docs →",
  },
  forgejo: {
    basePlaceholder: "https://codeberg.org or your instance",
    tokenPlaceholder: "access token",
    steps: [
      "Profile → Settings → Applications → Generate New Token.",
      "Grant repository read access, then paste the token here.",
      "Works with codeberg.org and self-hosted Forgejo alike.",
    ],
    docsUrl: "https://forgejo.org/docs/",
    docsLabel: "Forgejo docs →",
  },
  gitlab: {
    basePlaceholder: "https://gitlab.com (default)",
    tokenPlaceholder: "personal access token",
    steps: [
      "User Settings → Access Tokens → create one with the read_api scope.",
      "Self-hosted? Put the instance URL above, otherwise gitlab.com is used.",
    ],
    docsUrl: "https://docs.gitlab.com/ee/user/profile/personal_access_tokens.html",
    docsLabel: "GitLab token docs →",
  },
  bitbucket: {
    tokenPlaceholder: "OAuth key:secret or access token",
    workspace: true,
    steps: [
      "Workspace Settings → OAuth consumers → Add consumer (any callback URL).",
      "Grant Repositories: Read (plus Webhooks read/write for push deploys).",
      "Paste key:secret as shown, plus your workspace slug.",
    ],
    docsUrl: "https://support.atlassian.com/bitbucket-cloud/docs/use-oauth-on-bitbucket-cloud/",
    docsLabel: "Bitbucket OAuth docs →",
  },
};
