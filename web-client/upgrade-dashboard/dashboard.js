import init, * as Nimiq from '../dist/web/index.js';

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

// The staking contract lives at a fixed address on every Nimiq PoS network.
const STAKING_CONTRACT_ADDRESS = 'NQ77 0000 0000 0000 0000 0000 0000 0000 0001';

// Minimum support (in percent) required for BOTH stake and slots to trigger an upgrade.
// Mirrors `Policy::UPGRADE_MIN_SUPPORT` (primitives/src/policy.rs).
const UPGRADE_MIN_SUPPORT = 80;

// 1 NIM = 100000 Luna.
const LUNA_PER_NIM = 1e5;

// Re-fetch the (network-bound) validator data at most this often, in ms.
const REFRESH_INTERVAL_MS = 20_000;

// Optionally connect to a non-default network via `?network=testalbatross` (+ optional seeds),
// and/or pick a sync mode via `?sync=pico`. Without query params the client uses its built-in
// default network (MainAlbatross) and default sync mode (light). Light is the most widely
// supported mode by network peers; pico may fail to find peers on networks that don't serve it.
const params = new URLSearchParams(location.search);
const NETWORK = params.get('network');
const SYNC_MODE = params.get('sync'); // 'light' (default) | 'pico'
const SEED_NODES = [
    // For a custom network you typically need explicit seeds, e.g.:
    // "/dns4/seed1.pos.nimiq-testnet.com/tcp/8443/wss",
];

// ---------------------------------------------------------------------------
// State + small DOM helpers
// ---------------------------------------------------------------------------

const $ = (id) => document.getElementById(id);

const state = {
    targetVersion: null,
    rows: [],            // [{ address, stake, numSlots, signaled, supports }]
    totalStake: 0,       // Luna, over active validators
    supportingStake: 0,
    totalSlots: 0,       // over elected validators
    supportingSlots: 0,
    sortKey: 'numSlots',
    sortDir: -1,         // -1 desc, 1 asc
    refreshing: false,
    lastRefresh: 0,
};

const nim = (luna) => Math.round(luna / LUNA_PER_NIM).toLocaleString();
const pct = (part, total) => (total > 0 ? (part / total) * 100 : 0);
// Integer comparison mirroring the on-chain check: part*100 >= total*MIN.
const meets = (part, total) => part * 100 >= total * UPGRADE_MIN_SUPPORT;

// Decode the protocol version a validator signals: the first two bytes (big-endian u16)
// of the signal_data hash. This is the JS equivalent of `Policy::supports_upgrade`.
function signaledVersion(signalData) {
    if (!signalData) return null;
    const hex = signalData.startsWith('0x') ? signalData.slice(2) : signalData;
    if (hex.length < 4) return null;
    const v = parseInt(hex.slice(0, 4), 16);
    return Number.isNaN(v) ? null : v;
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

init().then(async () => {
    const config = new Nimiq.ClientConfiguration();
    config.logLevel('info');
    config.syncMode(SYNC_MODE || 'pico'); // pico by default; override with ?sync=light
    if (NETWORK) {
        config.network(NETWORK);
        if (SEED_NODES.length) config.seedNodes(SEED_NODES);
    }

    const client = await Nimiq.Client.create(config.build());
    window.client = client; // prevent GC / allow poking from the console

    client.addConsensusChangedListener((cstate) => {
        const el = $('consensus');
        el.textContent = cstate;
        el.classList.toggle('connecting', cstate === 'connecting');
        el.classList.toggle('syncing', cstate === 'syncing');
        el.classList.toggle('established', cstate === 'established');
        if (cstate === 'established') refreshData(client, true);
    });

    client.addPeerChangedListener((_id, _reason, numPeers) => {
        $('peers').textContent = numPeers;
    });

    let lastEpoch = null;
    client.addHeadChangedListener(async () => {
        const head = await updateHead(client);
        if (!head) return;
        // Refresh on every new epoch (validator set / slots change), otherwise throttle.
        if (head.epoch !== lastEpoch) {
            lastEpoch = head.epoch;
            refreshData(client);
        } else if (Date.now() - state.lastRefresh > REFRESH_INTERVAL_MS) {
            refreshData(client);
        }
    });

    // Periodic refresh so newly included signal transactions show up even on quiet chains.
    setInterval(() => {
        if (Date.now() - state.lastRefresh > REFRESH_INTERVAL_MS) refreshData(client);
    }, REFRESH_INTERVAL_MS);

    // Controls
    $('refresh').addEventListener('click', () => refreshData(client, true));
    $('search').addEventListener('input', renderTable);
    $('filter').addEventListener('change', renderTable);
    document.querySelectorAll('#validators th').forEach((th, i) => {
        const keys = ['address', 'stake', 'stake', 'numSlots', 'numSlots', 'signaled', 'supports'];
        th.addEventListener('click', () => {
            const key = keys[i];
            state.sortDir = state.sortKey === key ? -state.sortDir : -1;
            state.sortKey = key;
            renderTable();
        });
    });
});

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

// Cheap, local read of the head block: updates header + target version.
async function updateHead(client) {
    let head;
    try {
        head = await client.getHeadBlock();
    } catch (e) {
        return null;
    }
    state.targetVersion = head.version + 1;
    $('block').textContent = head.height;
    $('epoch').textContent = head.epoch;
    $('version').textContent = head.version;
    $('target-version').textContent = state.targetVersion;
    return head;
}

// Network-bound refresh of validators, stake and slots. Guarded against overlap.
async function refreshData(client, force = false) {
    if (state.refreshing) return;
    if (!force && !(await client.isConsensusEstablished())) return;
    state.refreshing = true;
    try {
        const head = await updateHead(client);
        if (!head) return;
        const target = state.targetVersion;

        // Stake side: the staking contract's active validators (address -> active stake in Luna).
        const staking = await client.getAccount(STAKING_CONTRACT_ADDRESS);
        const stakeByAddress = new Map(staking.activeValidators);

        // Slots side: the validators elected for the current epoch (address -> slot count).
        const elected = await client.getElectedValidators();
        const slotsByAddress = new Map(elected.map((v) => [v.address, v.numSlots]));

        // Signal data per validator (union of both sets).
        const addresses = [...new Set([...stakeByAddress.keys(), ...slotsByAddress.keys()])];
        const validators = await client.getValidators(addresses);
        const signalByAddress = new Map();
        addresses.forEach((addr, i) => {
            const v = validators[i];
            signalByAddress.set(addr, v ? signaledVersion(v.signalData) : null);
        });

        // Aggregate, mirroring tendermint.rs: stake over active_validators, slots over current_validators.
        let totalStake = 0, supportingStake = 0, totalSlots = 0, supportingSlots = 0;
        for (const [addr, balance] of stakeByAddress) {
            totalStake += balance;
            if (signalByAddress.get(addr) === target) supportingStake += balance;
        }
        for (const [addr, numSlots] of slotsByAddress) {
            totalSlots += numSlots;
            if (signalByAddress.get(addr) === target) supportingSlots += numSlots;
        }

        state.rows = addresses.map((addr) => {
            const signaled = signalByAddress.get(addr);
            return {
                address: addr,
                stake: stakeByAddress.get(addr) ?? 0,
                numSlots: slotsByAddress.get(addr) ?? 0,
                signaled,
                supports: signaled === target,
            };
        });
        Object.assign(state, { totalStake, supportingStake, totalSlots, supportingSlots });
        state.lastRefresh = Date.now();
        $('updated').textContent = new Date(state.lastRefresh).toLocaleTimeString();

        renderMetrics();
        renderTable();
    } catch (e) {
        console.error('Refresh failed:', e);
    } finally {
        state.refreshing = false;
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

function renderMetrics() {
    const { supportingStake, totalStake, supportingSlots, totalSlots, targetVersion } = state;
    const stakePct = pct(supportingStake, totalStake);
    const slotsPct = pct(supportingSlots, totalSlots);
    const stakeOk = meets(supportingStake, totalStake);
    const slotsOk = meets(supportingSlots, totalSlots);

    setMetric('stake', stakePct, stakeOk,
        `${nim(supportingStake)} of ${nim(totalStake)} NIM active stake`);
    setMetric('slots', slotsPct, slotsOk,
        `${supportingSlots} of ${totalSlots} slots`);

    const verdict = $('verdict');
    if (totalStake === 0 || totalSlots === 0) {
        verdict.className = 'verdict pending';
        verdict.textContent = 'Waiting for validator data…';
    } else if (stakeOk && slotsOk) {
        verdict.className = 'verdict ready';
        verdict.textContent = `✓ Upgrade to version ${targetVersion} would activate — both stake and slots meet the ${UPGRADE_MIN_SUPPORT}% threshold.`;
    } else {
        verdict.className = 'verdict not-ready';
        const missing = [!stakeOk && 'stake', !slotsOk && 'slots'].filter(Boolean).join(' and ');
        verdict.textContent = `✗ Upgrade to version ${targetVersion} not yet supported — ${missing} below ${UPGRADE_MIN_SUPPORT}%.`;
    }
}

function setMetric(prefix, percentage, ok, detail) {
    $(`${prefix}-pct`).textContent = percentage.toFixed(1);
    const bar = $(`${prefix}-bar`);
    bar.style.width = `${Math.min(percentage, 100)}%`;
    bar.classList.toggle('ok', ok);
    $(`${prefix}-detail`).textContent = detail;
    const badge = $(`${prefix}-badge`);
    badge.textContent = ok ? `✓ meets ${UPGRADE_MIN_SUPPORT}%` : `✗ below ${UPGRADE_MIN_SUPPORT}%`;
    badge.className = `card-badge ${ok ? 'ok' : 'no'}`;
}

function renderTable() {
    const body = $('validators-body');
    const query = $('search').value.trim().toUpperCase().replace(/\s/g, '');
    const filter = $('filter').value;
    const target = state.targetVersion;

    let rows = state.rows.filter((r) => {
        if (query && !r.address.replace(/\s/g, '').includes(query)) return false;
        if (filter === 'supporting') return r.supports;
        if (filter === 'other') return r.signaled !== null && !r.supports;
        if (filter === 'none') return r.signaled === null;
        return true;
    });

    const { sortKey, sortDir } = state;
    rows.sort((a, b) => {
        let av = a[sortKey], bv = b[sortKey];
        if (sortKey === 'address') return sortDir * String(av).localeCompare(String(bv));
        av = av ?? -1; bv = bv ?? -1; // nulls (no signal) sort last on desc
        return sortDir * (av - bv);
    });

    $('count').textContent = `${rows.length} of ${state.rows.length} validators`;

    if (!rows.length) {
        body.innerHTML = '<tr><td colspan="7" class="muted">No matching validators.</td></tr>';
        return;
    }

    body.innerHTML = rows.map((r) => {
        const stakeShare = pct(r.stake, state.totalStake).toFixed(2);
        const slotShare = pct(r.numSlots, state.totalSlots).toFixed(2);
        let tag;
        if (r.supports) tag = '<span class="tag ok">✓ yes</span>';
        else if (r.signaled !== null) tag = '<span class="tag other">other</span>';
        else tag = '<span class="tag no">no</span>';
        const signaled = r.signaled === null ? '–' : r.signaled;
        return `<tr class="${r.supports ? 'supporting' : ''}">
            <td class="addr" title="${r.address}">${r.address}</td>
            <td class="num">${nim(r.stake)}</td>
            <td class="num">${stakeShare}%</td>
            <td class="num">${r.numSlots}</td>
            <td class="num">${slotShare}%</td>
            <td class="num">${signaled}</td>
            <td>${tag}</td>
        </tr>`;
    }).join('');
}
