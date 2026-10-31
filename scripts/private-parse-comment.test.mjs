import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const workflow = readFileSync(new URL("../.github/workflows/coverage-parse-diff-comment.yml", import.meta.url), "utf8");
// Execute the shipped github-script body, with GitHub and artifact I/O stubbed.
const body = workflow.split("          script: |\n")[1].split("\n").map(line => line.replace(/^ {12}/, "")).join("\n");
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const script = new AsyncFunction("require", "core", "github", "context", body);
const head = "a".repeat(40);

async function run({prHead = head, repository = "bryanroscoe/yama-phase", artifactHead = head,
                    state = "open", comments = [], prNumber = "8"} = {}) {
  const writes = [];
  const files = {
    "parse-diff-artifact/pr-number.txt": prNumber,
    "parse-diff-artifact/parse-diff.md": `<!-- coverage-parse-diff -->\nGenerated for head ${artifactHead}\n`,
  };
  const require = name => {
    assert.equal(name, "fs");
    return {readFileSync: path => files[path]};
  };
  const issues = {
    listComments: () => {},
    createComment: async value => writes.push({kind: "create", ...value}),
    updateComment: async value => writes.push({kind: "update", ...value}),
  };
  const github = {
    rest: {issues, pulls: {get: async () => ({data: {state, head: {sha: prHead, repo: {full_name: repository}}}})}},
    paginate: async () => comments,
  };
  await script(require, {info: () => {}}, github, {
    repo: {owner: "bryanroscoe", repo: "yama-phase"},
    payload: {workflow_run: {head_sha: head}},
  });
  return writes;
}

test("current private head posts evidence, including a zero-change result", async () => {
  const writes = await run();
  assert.equal(writes.length, 1);
  assert.equal(writes[0].kind, "create");
  assert.equal(writes[0].issue_number, 8);
  assert.ok(writes[0].body.includes(head));
});

test("slow stale runs, foreign repositories, closed PRs and headless artifacts cannot post", async () => {
  for (const args of [
    {prHead: "b".repeat(40)}, {repository: "other/fork"}, {state: "closed"},
    {artifactHead: "unknown"}, {prNumber: "0"}, {prNumber: "NaN"}, {prNumber: "8garbage"},
  ]) assert.deepEqual(await run(args), []);
});

test("only the bot's sticky is updated; a user's marked comment is preserved", async () => {
  const user = {id: 1, user: {login: "bryanroscoe"}, body: "<!-- coverage-parse-diff -->"};
  const bot = {id: 2, user: {login: "github-actions[bot]"}, body: "<!-- coverage-parse-diff -->"};
  const writes = await run({comments: [user, bot]});
  assert.equal(writes.length, 1);
  assert.equal(writes[0].kind, "update");
  assert.equal(writes[0].comment_id, 2);
});
