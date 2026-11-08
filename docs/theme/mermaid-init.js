// Mermaid initialization hook for AndroGrid mdBook
// Traces to: [NFR-DOC-001], [REQ-PUB-001]
window.initializeAndroGridMermaid = function() {
    if (!window.mermaid) {
        return;
    }
    window.mermaid.initialize({
        startOnLoad: false,
        theme: 'default',
        securityLevel: 'loose'
    });

    var codeElements = document.querySelectorAll('pre code.language-mermaid');
    codeElements.forEach(function(codeEl, index) {
        var preEl = codeEl.parentElement;
        var insertDiv = document.createElement('div');
        insertDiv.className = 'mermaid';
        insertDiv.textContent = codeEl.textContent;
        preEl.parentNode.insertBefore(insertDiv, preEl);
        preEl.style.display = 'none';
    });

    window.mermaid.run();
};

document.addEventListener('DOMContentLoaded', function() {
    if (window.mermaid) {
        window.initializeAndroGridMermaid();
    }
});
