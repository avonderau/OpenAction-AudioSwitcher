// Settings page for the Audio Switcher Deck plugin (Stream Deck property
// inspector API: the host calls connectElgatoStreamDeckSocket).
(function () {
  'use strict';
  const STATUS_TEXT = {
    starting: ['Looking for the audio server…', ''],
    noserver: ['pactl cannot reach PulseAudio or PipeWire (pipewire-pulse).', 'bad'],
    nopactl: ['pactl is not installed. Install pulseaudio-utils (Debian, Ubuntu, Fedora) or libpulse (Arch).', 'bad'],
  };

  const DEFAULT_KEEP = 'TeamSpeak, Zoom, Discord';
  let ws = null;
  let uuid = '';
  let action = '';
  let settings = {};
  let last = null;
  let waiting = null;
  const $ = (id) => document.getElementById(id);

  function send(event, payload) {
    if (ws && ws.readyState === 1) ws.send(JSON.stringify({ event, context: uuid, action, payload }));
  }

  // expectReply warns when the plugin does not answer, which usually means
  // its process is not running (see the key's title and the plugin log).
  function expectReply(text) {
    const el = $('status');
    el.textContent = text;
    el.className = 'status';
    el.hidden = false;
    clearTimeout(waiting);
    waiting = setTimeout(function () {
      el.textContent = 'The plugin is not responding. If the key says "Plugin crashed" or "Plugin host offline", check the plugin log in ~/.local/state/OpenLinkHub/deck-plugins/logs/.';
      el.className = 'status bad';
    }, 4000);
  }

  function chosen() {
    return Array.isArray(settings.devices) ? settings.devices.filter((d) => typeof d === 'string') : [];
  }

  function labels() {
    return settings.labels && typeof settings.labels === 'object' ? settings.labels : {};
  }

  // Descriptions are saved with the key so a device that comes back
  // under a new name can still be recognised.
  function save() {
    const devices = [];
    const names = {};
    const old = settings.descriptions && typeof settings.descriptions === 'object' ? settings.descriptions : {};
    const descriptions = {};
    const known = new Map(((last && last.devices) || []).map((d) => [d.name, d.description]));
    for (const li of $('devices').children) {
      const name = li.dataset.name;
      if (li.querySelector('input[type=checkbox]').checked) {
        devices.push(name);
        const desc = old[name] || known.get(name) || known.get(((last && last.matches) || {})[name]);
        if (desc) descriptions[name] = desc;
      }
      const label = li.querySelector('input.label').value.trim();
      if (label) names[name] = label;
    }
    settings.devices = devices;
    settings.labels = names;
    settings.descriptions = descriptions;
    settings.matchSimilar = $('matchSimilar').checked;
    settings.moveApps = $('moveApps').checked;
    settings.keepApps = $('keepApps').value;
    $('keepBox').hidden = !settings.moveApps;
    send('setSettings', settings);
  }

  // Ticked devices first, in their saved order (missing ones included, so
  // an unplugged headset keeps its place), then the rest as pactl lists them.
  function render(p) {
    const list = $('devices');
    list.textContent = '';
    const known = new Map((p.devices || []).map((d) => [d.name, d.description]));
    const picked = chosen();
    const matches = p.matches || {};
    const stood = Object.values(matches);
    const saved = settings.descriptions || {};
    const rows = picked.concat((p.devices || []).map((d) => d.name).filter((n) => !picked.includes(n) && !stood.includes(n)));
    for (const name of rows) {
      const li = document.createElement('li');
      li.dataset.name = name;
      const box = document.createElement('input');
      box.type = 'checkbox';
      box.checked = picked.includes(name);
      box.addEventListener('change', save);
      const title = document.createElement('label');
      title.className = 'name';
      title.textContent = known.get(name) || known.get(matches[name]) || saved[name] || name;
      title.title = name;
      if (name === p.current || (matches[name] && matches[name] === p.current)) {
        const tag = document.createElement('span');
        tag.className = 'current';
        tag.textContent = ' · in use';
        title.appendChild(tag);
      } else if (matches[name]) {
        const tag = document.createElement('span');
        tag.className = 'missing';
        tag.textContent = ' · found under a new name';
        title.appendChild(tag);
        title.title = name + ' → ' + matches[name];
      } else if (!known.has(name)) {
        const tag = document.createElement('span');
        tag.className = 'missing';
        tag.textContent = ' · not connected';
        title.appendChild(tag);
      }
      title.addEventListener('click', function () { box.click(); });
      const label = document.createElement('input');
      label.className = 'label';
      label.placeholder = 'Key label';
      label.maxLength = 20;
      label.value = labels()[name] || '';
      label.addEventListener('change', save);
      li.append(box, title, label);
      list.appendChild(li);
    }
  }

  function showDevices(p) {
    clearTimeout(waiting);
    last = p;
    $('heading').textContent = p.kind === 'input' ? 'Microphones' : 'Speakers and headphones';
    const el = $('status');
    if (p.status === 'ready') {
      el.hidden = true;
    } else {
      const [text, cls] = STATUS_TEXT[p.status] || ['Unknown state', ''];
      el.hidden = false;
      el.textContent = text;
      el.className = 'status ' + cls;
    }
    // Don't rebuild the list under the cursor while a label is being typed.
    if (!$('devices').contains(document.activeElement)) render(p);
  }

  window.connectElgatoStreamDeckSocket = function (port, inUUID, registerEvent, info, actionInfo) {
    uuid = inUUID;
    try {
      const a = JSON.parse(actionInfo || '{}');
      action = a.action || '';
      settings = (a.payload && a.payload.settings) || {};
    } catch (e) { /* keep defaults */ }
    $('matchSimilar').checked = settings.matchSimilar !== false;
    $('matchSimilar').addEventListener('change', save);
    $('moveApps').checked = settings.moveApps === true;
    $('keepApps').value = typeof settings.keepApps === 'string' ? settings.keepApps : DEFAULT_KEEP;
    $('keepBox').hidden = !$('moveApps').checked;
    $('moveApps').addEventListener('change', save);
    $('keepApps').addEventListener('change', save);
    ws = new WebSocket('ws://127.0.0.1:' + port);
    ws.onopen = function () {
      ws.send(JSON.stringify({ event: registerEvent, uuid }));
      send('sendToPlugin', { type: 'devices' });
      expectReply('Checking…');
    };
    ws.onmessage = function (evt) {
      let msg;
      try { msg = JSON.parse(evt.data); } catch (e) { return; }
      if (msg.event === 'sendToPropertyInspector' && msg.payload && msg.payload.type === 'devices') {
        showDevices(msg.payload);
      } else if (msg.event === 'didReceiveSettings' && msg.payload) {
        settings = msg.payload.settings || {};
        if (last) showDevices(last);
      }
    };
  };
  window.connectSocket = window.connectElgatoStreamDeckSocket;

  $('refresh').addEventListener('click', function () {
    send('sendToPlugin', { type: 'refresh' });
    send('sendToPlugin', { type: 'devices' });
    expectReply('Refreshing…');
  });
})();
