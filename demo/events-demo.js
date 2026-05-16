document.title = "Tobira Event Plumbing Ready";

const log = document.getElementById("log");
const input = document.getElementById("demo-input");
const form = document.getElementById("demo-form");
const link = document.getElementById("demo-link");
const submit = document.getElementById("demo-submit");

function nodeLabel(node) {
  if (!node) {
    return "null";
  }
  if (node.id) {
    return `#${node.id}`;
  }
  if (node.tagName) {
    return node.tagName.toLowerCase();
  }
  if (node === document) {
    return "document";
  }
  return "node";
}

function appendLine(line) {
  log.textContent += `${line}\n`;
}

function describeEvent(event) {
  return [
    `type=${event.type}`,
    `target=${nodeLabel(event.target)}`,
    `current=${nodeLabel(event.currentTarget)}`,
    `defaultPrevented=${event.defaultPrevented}`,
  ].join(" ");
}

function record(scope, event) {
  appendLine(`${scope}: ${describeEvent(event)}`);
}

document.addEventListener("DOMContentLoaded", (event) => {
  record("document", event);
});

document.addEventListener("focus", (event) => {
  record("document", event);
});

document.addEventListener("blur", (event) => {
  record("document", event);
});

document.addEventListener("input", (event) => {
  record("document", event);
});

document.addEventListener("change", (event) => {
  record("document", event);
});

document.addEventListener("click", (event) => {
  record("document", event);
});

document.addEventListener("submit", (event) => {
  record("document", event);
});

input.addEventListener("focus", (event) => {
  record("input", event);
});

input.addEventListener("blur", (event) => {
  record("input", event);
});

input.addEventListener("input", (event) => {
  record("input", event);
});

input.addEventListener("change", (event) => {
  record("input", event);
});

submit.addEventListener("click", (event) => {
  record("submit", event);
});

form.addEventListener("submit", (event) => {
  record("form", event);
  event.preventDefault();
  appendLine("form: preventDefault() called");
});

link.addEventListener("click", (event) => {
  record("link", event);
  event.preventDefault();
  appendLine("link: preventDefault() called");
});

appendLine("script: listeners attached");
