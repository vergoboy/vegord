/* vegord Debug Panel renderer logic (plain JS, no bundler). */

const logEl = document.getElementById("log");
const countEl = document.getElementById("log-count");
const autoscrollEl = document.getElementById("autoscroll");
const details = {
    proxy: document.getElementById("st-proxy"),
    doh: document.getElementById("st-doh"),
    ip: document.getElementById("st-ip"),
    conn: document.getElementById("st-conn"),
    query: document.getElementById("st-query"),
    traffic: document.getElementById("st-traffic"),
    detailDoh: document.getElementById("detail-doh"),
    detailIps: document.getElementById("detail-ips")
};

let totalLines = 0;

function classify(line) {
    if (/\[[^\]]+:error\]/.test(line)) return "error";
    if (/\[[^\]]+:warn\]/.test(line)) return "warn";
    if (/\[proxy:/.test(line)) return "proxy";
    if (/\[RENDERER:/.test(line)) return "renderer";
    if (/\[NETQUALITY:/.test(line)) return "netquality";
    return "";
}

function appendLine(line) {
    const div = document.createElement("div");
    div.className = "l " + classify(line);
    div.textContent = line;
    logEl.appendChild(div);
    while (logEl.childElementCount > 3000) logEl.removeChild(logEl.firstChild);
    totalLines++;
    countEl.textContent = totalLines + " lines";
    if (autoscrollEl.checked) logEl.scrollTop = logEl.scrollHeight;
}

function fmtBytes(n) {
    if (n == null) return "-";
    if (n < 1024) return n + " B";
    if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
    return (n / 1024 / 1024).toFixed(2) + " MB";
}

function fmtMs(n) {
    return n == null ? "?" : Math.round(n) + " ms";
}

function card(elm, ok) {
    elm.classList.toggle("ok", ok === true);
    elm.classList.toggle("bad", ok === false);
}

function renderStatus(st) {
    if (!st) {
        card(details.proxy, false);
        details.proxy.textContent = "unavailable";
        details.doh.textContent = "-";
        details.ip.textContent = "-";
        details.conn.textContent = "-";
        details.query.textContent = "-";
        details.traffic.textContent = "-";
        details.detailDoh.textContent = "";
        details.detailIps.textContent = "";
        return;
    }

    card(details.proxy, st.ok);
    details.proxy.textContent = st.ok ? "healthy" : "unhealthy";

    const currentProbe = (st.probeResults || []).find(r => r.index === st.currentDohIndex);
    details.doh.textContent =
        "#" + st.currentDohIndex +
        " (" + fmtMs(currentProbe && currentProbe.avgRttMs) + ")" +
        " switches=" + st.totalSwitches;
    card(details.doh, st.currentDoh ? true : null);

    details.ip.textContent = st.discordBestIp
        ? st.discordBestIp + " " + fmtMs(st.discordBestRtt)
        : "none";
    card(details.ip, st.discordBestIp ? true : null);

    const conn = st.connections;
    if (conn) {
        details.conn.textContent = conn.total + " total, " + conn.ok + " ok, " + conn.filtered + " filtered";
        card(details.conn, conn.total > 0);
    } else {
        details.conn.textContent = "-";
    }

    const q = st.queries;
    if (q) {
        details.query.textContent = q.total + " total, " + q.ok + " ok, " + q.fail + " fail";
        card(details.query, q.fail === 0 ? (q.total > 0 ? true : null) : false);
    } else {
        details.query.textContent = "-";
    }

    const t = st.traffic;
    if (t) {
        details.traffic.textContent = "up " + fmtBytes(t.ulBytes) + " / down " + fmtBytes(t.dlBytes);
    } else {
        details.traffic.textContent = "-";
    }

    const dohLines = ["current: " + (st.currentDoh || "-")];
    const sorted = (st.probeResults || [])
        .filter(r => r.avgRttMs != null)
        .sort((a, b) => a.avgRttMs - b.avgRttMs);
    sorted.forEach((r, i) => {
        dohLines.push((i === 0 ? "best  " : "  #" + r.index + " ") + fmtMs(r.avgRttMs) + " " + r.url);
    });
    const fails = (st.probeResults || []).filter(r => r.avgRttMs == null).map(r => "#" + r.index);
    if (fails.length) dohLines.push("fail: " + fails.join(", "));
    details.detailDoh.textContent = dohLines.join("\n");

    const ipLines = (st.discordIps || []).map(s => {
        const loss = s.lossPct != null ? s.lossPct.toFixed(1) + "%" : "?";
        return s.ip.padEnd(16) + fmtMs(s.rttMs) + "  loss " + loss;
    });
    details.detailIps.textContent = ipLines.join("\n") || "no Discord IPs yet";
}

async function pollStatus() {
    try {
        renderStatus(await window.vegordDebug.getStatus());
    } catch {
        renderStatus(null);
    }
    setTimeout(pollStatus, 2000);
}

document.addEventListener("DOMContentLoaded", () => {
    window.vegordDebug.onLogLine(appendLine);
    window.vegordDebug.getRecentLogs().then(lines => {
        for (const line of lines) appendLine(line);
    });

    document.getElementById("btn-rescan").addEventListener("click", () => {
        window.vegordDebug.rescanDoH();
        appendLine("[DEBUG] manual DoH rescan requested");
    });
    document.getElementById("btn-clear").addEventListener("click", async () => {
        await window.vegordDebug.clearLog();
        logEl.textContent = "";
        totalLines = 0;
        countEl.textContent = "";
    });
    document.getElementById("btn-copy").addEventListener("click", () => {
        window.vegordDebug.copyLog(logEl.textContent);
    });
    document.getElementById("btn-close").addEventListener("click", () => window.close());

    pollStatus();
});
