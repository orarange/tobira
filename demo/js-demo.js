document.writeln('<div class="card"><strong>external script:</strong> loaded via &lt;script src&gt;.</div>');
setTimeout(function () {
  document.write('<div class="card"><strong>timeout:</strong> callback executed.</div>');
}, 1);
