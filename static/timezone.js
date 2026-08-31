(() => {
    const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (!timezone) return;
    fetch("/timezone", {
        method: "POST",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify({timezone}),
    }).then(response => {
        if (response.ok) window.location.reload();
    });
})();
