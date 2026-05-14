var results = document.getElementById("results");
var summary = document.querySelector("#summary-card");
var playground = document.getElementById("playground");
var writeLog = document.getElementById("write-log");

summary.textContent = "Boot: inline script, external script, selector queries, and DOM mutations all ran.";

var firstCard = document.createElement("article");
firstCard.className = "card pass";
firstCard.innerHTML = "<strong>Selector check:</strong> document.querySelector and getElementById returned live nodes.";
results.appendChild(firstCard);

var secondCard = document.createElement("article");
secondCard.className = "card pass";
secondCard.textContent = "Mutation check: createElement, textContent, appendChild, and className worked.";
results.appendChild(secondCard);

var inserted = document.createElement("article");
inserted.className = "card info";
inserted.innerHTML = "<strong>Insert check:</strong> this card was inserted before the mutation card.";
results.insertBefore(inserted, secondCard);

playground.classList.remove("info");
playground.classList.add("pass");
playground.innerHTML =
  "<strong>innerHTML check:</strong> replaced a placeholder node with richer markup. " +
  "<span class=\"inline\">classList</span> also updated the visual state.";

var tags = document.getElementById("tags");
var extraTag = document.createElement("span");
extraTag.className = "tag";
extraTag.textContent = "insertBefore";
tags.appendChild(extraTag);

var transient = document.createElement("div");
transient.className = "card warn";
transient.textContent = "This node will remove itself right away.";
results.appendChild(transient);
transient.remove();

writeLog.className = "card info";
