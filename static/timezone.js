(() => {
    const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (!timezone) return;
    const csrf_token = document.querySelector('meta[name="csrf-token"]').content;
    fetch("/timezone", {
        method: "POST",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify({timezone, csrf_token}),
    }).then(response => {
        if (response.ok) window.location.reload();
    });
})();
