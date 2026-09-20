// Doubles what it is sent, and says hello once on its own.
self.postMessage("ready");
self.onmessage = function (event) {
  self.postMessage({ doubled: event.data.n * 2, from: "worker" });
};
