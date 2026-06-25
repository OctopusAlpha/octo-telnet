/*
 * Octo-Telnet: Terminal emulator and WebSocket client
 *
 * Implements:
 * - WebSocket connection to the Rust backend
 * - VT100/ANSI terminal emulator with character grid
 * - ANSI escape code parsing (SGR colors, cursor movement, erase)
 * - Keyboard input handling with special key sequences
 */

(function () {
    'use strict';

    // --- Configuration ---
    const COLS = 80;
    const ROWS = 25;
    const RENDER_INTERVAL = 16; // ~60fps cap for rendering

    // --- DOM elements ---
    const output = document.getElementById('terminal-output');
    const cursor = document.getElementById('cursor');
    const hostInput = document.getElementById('host-input');
    const connectBtn = document.getElementById('connect-btn');
    const disconnectBtn = document.getElementById('disconnect-btn');
    const statusIndicator = document.getElementById('status-indicator');
    const connectionInfo = document.getElementById('connection-info');

    // --- WebSocket state ---
    let ws = null;
    let connected = false;

    // --- Terminal state ---
    // Current text attributes (must be defined before grid, since makeCell references it)
    let attr = {
        fg: 7,
        bg: 0,
        bold: false,
        underline: false,
        reverse: false
    };

    let grid = createGrid();
    let cursorX = 0;
    let cursorY = 0;
    let savedCursorX = 0;
    let savedCursorY = 0;
    let cursorVisible = true;

    // ANSI parser state
    let ansiState = 'normal'; // 'normal' | 'escape' | 'csi'
    let ansiBuffer = '';
    let renderPending = false;

    // ======================================================================
    // Grid management
    // ======================================================================

    function createGrid() {
        const g = [];
        for (let y = 0; y < ROWS; y++) {
            const row = [];
            for (let x = 0; x < COLS; x++) {
                row.push(makeCell());
            }
            g.push(row);
        }
        return g;
    }

    function makeCell() {
        return {
            ch: ' ',
            fg: attr.fg,
            bg: attr.bg,
            bold: attr.bold,
            underline: attr.underline,
            reverse: attr.reverse,
            width: 1
        };
    }

    function clearGrid() {
        for (let y = 0; y < ROWS; y++) {
            for (let x = 0; x < COLS; x++) {
                grid[y][x] = makeCell();
            }
        }
    }

    function scrollUp() {
        grid.shift();
        const newRow = [];
        for (let x = 0; x < COLS; x++) {
            newRow.push(makeCell());
        }
        grid.push(newRow);
    }

    // ======================================================================
    // Character output
    // ======================================================================

    // Output a character at the current cursor position with specified width.
    // Double-width CJK characters occupy 2 grid cells.
    function putChar(ch, width) {
        width = width || 1;
        if (width > COLS) width = COLS;

        if (cursorX + width > COLS) {
            cursorX = 0;
            cursorY++;
            if (cursorY >= ROWS) {
                scrollUp();
                cursorY = ROWS - 1;
            }
        }
        if (cursorY >= ROWS) {
            scrollUp();
            cursorY = ROWS - 1;
        }

        grid[cursorY][cursorX] = {
            ch: ch,
            fg: attr.fg,
            bg: attr.bg,
            bold: attr.bold,
            underline: attr.underline,
            reverse: attr.reverse,
            width: width
        };

        // Mark continuation cells for double-width characters
        for (let i = 1; i < width; i++) {
            if (cursorX + i < COLS) {
                grid[cursorY][cursorX + i] = {
                    ch: '',
                    fg: attr.fg,
                    bg: attr.bg,
                    bold: false,
                    underline: false,
                    reverse: false,
                    width: 0
                };
            }
        }

        cursorX += width;
    }

    function newline() {
        cursorX = 0;
        cursorY++;
        if (cursorY >= ROWS) {
            scrollUp();
            cursorY = ROWS - 1;
        }
    }

    function carriageReturn() {
        cursorX = 0;
    }

    function backspace() {
        if (cursorX > 0) {
            cursorX--;
        }
    }

    function tab() {
        const next = Math.floor(cursorX / 8) * 8 + 8;
        cursorX = Math.min(next, COLS - 1);
    }

    // ======================================================================
    // Character encoding: configurable UTF-8 / GBK / CP437
    // Text bytes are accumulated and decoded in chunks using the selected encoding.
    // ANSI control codes are handled at the byte level.
    // ======================================================================

    // CP437 to Unicode mapping for bytes 0x80-0xFF
    var CP437 = [
        '\u00C7','\u00FC','\u00E9','\u00E2','\u00E4','\u00E0','\u00E5','\u00E7',
        '\u00EA','\u00EB','\u00E8','\u00EF','\u00EE','\u00EC','\u00C4','\u00C5',
        '\u00C9','\u00E6','\u00C6','\u00F4','\u00F6','\u00F2','\u00FB','\u00F9',
        '\u00FF','\u00D6','\u00DC','\u00A2','\u00A3','\u00A5','\u20A7','\u0192',
        '\u00E1','\u00ED','\u00F3','\u00FA','\u00F1','\u00D1','\u00AA','\u00BA',
        '\u00BF','\u2310','\u00AC','\u00BD','\u00BC','\u00A1','\u00AB','\u00BB',
        '\u2591','\u2592','\u2593','\u2502','\u2524','\u2561','\u2562','\u2556',
        '\u2555','\u2563','\u2551','\u2557','\u255D','\u255C','\u255B','\u2510',
        '\u2514','\u2534','\u252C','\u251C','\u2500','\u253C','\u255E','\u255F',
        '\u255A','\u2554','\u2569','\u2566','\u2560','\u2550','\u256C','\u2567',
        '\u2568','\u2564','\u2565','\u2559','\u2558','\u2552','\u2553','\u256B',
        '\u256A','\u2518','\u250C','\u2588','\u2584','\u258C','\u2590','\u2580',
        '\u03B1','\u00DF','\u0393','\u03C0','\u03A3','\u03C3','\u00B5','\u03C4',
        '\u03A6','\u0398','\u03A9','\u03B4','\u221E','\u2205','\u2208','\u2229',
        '\u2261','\u00B1','\u2265','\u2264','\u2320','\u2321','\u00F7','\u2248',
        '\u00B0','\u2219','\u00B7','\u221A','\u207F','\u00B2','\u25A0','\u00A0'
    ];

    // Current text encoding and accumulation buffer
    var textEncoding = 'utf-8';
    var textBuffer = [];
    var encodingLabel = document.getElementById('encoding');

    function setEncoding(enc) {
        textEncoding = enc;
        textBuffer = [];
        resetTerminal();
        encodingLabel.textContent = enc.toUpperCase() + ' / VT100';
    }

    function decodeCP437(bytes) {
        var s = '';
        for (var i = 0; i < bytes.length; i++) {
            var b = bytes[i];
            s += b < 0x80 ? String.fromCharCode(b) : CP437[b - 0x80];
        }
        return s;
    }

    // Flush accumulated text bytes and decode using the selected encoding
    function flushTextBuffer() {
        if (textBuffer.length === 0) return;
        var bytes = new Uint8Array(textBuffer);
        textBuffer = [];

        var text;
        if (textEncoding === 'cp437') {
            text = decodeCP437(bytes);
        } else if (textEncoding === 'gbk' || textEncoding === 'gb2312') {
            text = new TextDecoder('gbk', { fatal: false }).decode(bytes);
        } else {
            text = new TextDecoder('utf-8', { fatal: false }).decode(bytes);
        }

        for (var i = 0; i < text.length; i++) {
            var cp = text.charCodeAt(i);
            var w = (cp >= 0x1100 && cp <= 0x115F) ||
                    (cp >= 0x2E80 && cp <= 0x9FFF) ||
                    (cp >= 0xA000 && cp <= 0xA4FF) ||
                    (cp >= 0xAC00 && cp <= 0xD7AF) ||
                    (cp >= 0xF900 && cp <= 0xFAFF) ||
                    (cp >= 0xFE30 && cp <= 0xFE6F) ||
                    (cp >= 0xFF01 && cp <= 0xFF60) ||
                    (cp >= 0x20000 && cp <= 0x2FFFF) ? 2 : 1;
            putChar(text[i], w);
        }
    }

    // ======================================================================
    // ANSI escape code parser (byte-level, text buffered per encoding)
    // ======================================================================

    function processData(data) {
        for (var i = 0; i < data.length; i++) {
            var byte = data[i];

            switch (ansiState) {
                case 'normal':
                    if (byte === 0x1b) {
                        flushTextBuffer();
                        ansiState = 'escape';
                        ansiBuffer = '';
                    } else if (byte === 0x0d) {
                        flushTextBuffer();
                        carriageReturn();
                    } else if (byte === 0x0a) {
                        flushTextBuffer();
                        newline();
                    } else if (byte === 0x08) {
                        flushTextBuffer();
                        backspace();
                    } else if (byte === 0x09) {
                        flushTextBuffer();
                        tab();
                    } else if (byte === 0x07) {
                        flushTextBuffer();
                    } else if (byte >= 0x20) {
                        textBuffer.push(byte);
                        if (textBuffer.length >= 4096) flushTextBuffer();
                    }
                    break;

                case 'escape':
                    if (byte === 0x5b) {
                        ansiState = 'csi';
                        ansiBuffer = '';
                    } else if (byte === 0x5d) {
                        ansiState = 'osc';
                        ansiBuffer = '';
                    } else if (byte === 0x37) {
                        savedCursorX = cursorX;
                        savedCursorY = cursorY;
                        ansiState = 'normal';
                    } else if (byte === 0x38) {
                        cursorX = savedCursorX;
                        cursorY = savedCursorY;
                        ansiState = 'normal';
                    } else if (byte === 0x63) {
                        flushTextBuffer();
                        resetTerminal();
                        ansiState = 'normal';
                    } else if (byte === 0x4d) {
                        if (cursorY > 0) cursorY--;
                        ansiState = 'normal';
                    } else if (byte === 0x44) {
                        newline();
                        ansiState = 'normal';
                    } else if (byte === 0x45) {
                        cursorX = 0;
                        newline();
                        ansiState = 'normal';
                    } else if (byte === 0x28 || byte === 0x29) {
                        ansiState = 'charset';
                    } else {
                        ansiState = 'normal';
                    }
                    break;

                case 'csi':
                    if (byte >= 0x40 && byte <= 0x7e) {
                        handleCSI(ansiBuffer, String.fromCharCode(byte));
                        ansiState = 'normal';
                        ansiBuffer = '';
                    } else if (byte >= 0x20 && byte <= 0x3f) {
                        ansiBuffer += String.fromCharCode(byte);
                    } else {
                        ansiState = 'normal';
                    }
                    break;

                case 'osc':
                    if (byte === 0x07) {
                        ansiState = 'normal';
                    } else if (byte === 0x1b) {
                        ansiState = 'osc_esc';
                    }
                    break;

                case 'osc_esc':
                    if (byte === 0x5c) {
                        ansiState = 'normal';
                    } else {
                        ansiState = 'escape';
                        ansiBuffer = '';
                        continue;
                    }
                    break;

                case 'charset':
                    ansiState = 'normal';
                    break;
            }
        }

        requestRender();
    }

    function handleCSI(params, final) {
        // Parse parameters
        const parts = params.replace(/^\?/, '').split(';');
        const isPrivate = params.startsWith('?');
        const nums = parts.map(p => parseInt(p, 10) || 0);
        const n0 = nums[0] || 0;
        const n1 = nums[1] || 0;

        switch (final) {
            case 'm': // SGR (Select Graphic Rendition)
                handleSGR(nums);
                break;

            case 'A': // Cursor Up
                cursorY = Math.max(0, cursorY - (n0 || 1));
                break;

            case 'B': // Cursor Down
                cursorY = Math.min(ROWS - 1, cursorY + (n0 || 1));
                break;

            case 'C': // Cursor Forward
                cursorX = Math.min(COLS - 1, cursorX + (n0 || 1));
                break;

            case 'D': // Cursor Back
                cursorX = Math.max(0, cursorX - (n0 || 1));
                break;

            case 'E': // Cursor Next Line
                cursorY = Math.min(ROWS - 1, cursorY + (n0 || 1));
                cursorX = 0;
                break;

            case 'F': // Cursor Previous Line
                cursorY = Math.max(0, cursorY - (n0 || 1));
                cursorX = 0;
                break;

            case 'G': // Cursor Horizontal Absolute
            case '`':
                cursorX = Math.min(COLS - 1, Math.max(0, n0 - 1));
                break;

            case 'd': // Cursor Vertical Absolute
                cursorY = Math.min(ROWS - 1, Math.max(0, n0 - 1));
                break;

            case 'H': // Cursor Position
            case 'f':
                cursorY = Math.min(ROWS - 1, Math.max(0, (n0 || 1) - 1));
                cursorX = Math.min(COLS - 1, Math.max(0, (n1 || 1) - 1));
                break;

            case 'J': // Erase Display
                eraseDisplay(n0);
                break;

            case 'K': // Erase Line
                eraseLine(n0);
                break;

            case 's': // Save cursor
                savedCursorX = cursorX;
                savedCursorY = cursorY;
                break;

            case 'u': // Restore cursor
                cursorX = savedCursorX;
                cursorY = savedCursorY;
                break;

            case 'h': // Set Mode
                if (isPrivate && n0 === 25) {
                    cursorVisible = true;
                }
                break;

            case 'l': // Reset Mode
                if (isPrivate && n0 === 25) {
                    cursorVisible = false;
                }
                break;

            case 'r': // Set Scrolling Region (ignored -- we use full screen)
                break;

            case 'L': // Insert Lines
                insertLines(n0 || 1);
                break;

            case 'M': // Delete Lines
                deleteLines(n0 || 1);
                break;

            case 'P': // Delete Characters
                deleteChars(n0 || 1);
                break;

            case '@': // Insert Characters
                insertChars(n0 || 1);
                break;

            default:
                // Unknown CSI -- ignore
                break;
        }
    }

    function handleSGR(nums) {
        if (nums.length === 0) {
            nums = [0];
        }

        let i = 0;
        while (i < nums.length) {
            const n = nums[i];
            switch (n) {
                case 0: // Reset all attributes
                    attr.fg = 7;
                    attr.bg = 0;
                    attr.bold = false;
                    attr.underline = false;
                    attr.reverse = false;
                    break;
                case 1: // Bold
                    attr.bold = true;
                    break;
                case 4: // Underline
                    attr.underline = true;
                    break;
                case 7: // Reverse video
                    attr.reverse = true;
                    break;
                case 22: // Normal intensity (not bold)
                    attr.bold = false;
                    break;
                case 24: // Not underlined
                    attr.underline = false;
                    break;
                case 27: // Not reversed
                    attr.reverse = false;
                    break;
                case 30: case 31: case 32: case 33:
                case 34: case 35: case 36: case 37:
                    // Standard foreground colors
                    attr.fg = n - 30;
                    break;
                case 38:
                    // Extended foreground color (256-color or true color)
                    if (nums[i + 1] === 5) {
                        attr.fg = map256(nums[i + 2]);
                        i += 2;
                    } else if (nums[i + 1] === 2) {
                        // True color -- approximate to 16-color
                        attr.fg = approximateColor(nums[i + 2], nums[i + 3], nums[i + 4]);
                        i += 4;
                    }
                    break;
                case 39: // Default foreground
                    attr.fg = 7;
                    break;
                case 40: case 41: case 42: case 43:
                case 44: case 45: case 46: case 47:
                    // Standard background colors
                    attr.bg = n - 40;
                    break;
                case 48:
                    // Extended background color
                    if (nums[i + 1] === 5) {
                        attr.bg = map256(nums[i + 2]);
                        i += 2;
                    } else if (nums[i + 1] === 2) {
                        attr.bg = approximateColor(nums[i + 2], nums[i + 3], nums[i + 4]);
                        i += 4;
                    }
                    break;
                case 49: // Default background
                    attr.bg = 0;
                    break;
                case 90: case 91: case 92: case 93:
                case 94: case 95: case 96: case 97:
                    // Bright foreground colors
                    attr.fg = n - 90 + 8;
                    break;
                case 100: case 101: case 102: case 103:
                case 104: case 105: case 106: case 107:
                    // Bright background colors
                    attr.bg = n - 100 + 8;
                    break;
            }
            i++;
        }
    }

    // Map 256-color palette to 16-color palette
    function map256(n) {
        if (n < 8) return n;
        if (n < 16) return n;
        // 216-color cube and grayscale -- approximate
        if (n >= 232) {
            // Grayscale: map to bright black (8) or white (7/15)
            return n >= 248 ? 15 : 8;
        }
        // 6x6x6 color cube
        const r = Math.floor((n - 16) / 36) % 6;
        const g = Math.floor((n - 16) / 6) % 6;
        const b = (n - 16) % 6;
        return approximateColor(r * 51, g * 51, b * 51);
    }

    // Approximate RGB to 16-color palette
    function approximateColor(r, g, b) {
        const colors = [
            [0, 0, 0], [205, 0, 0], [0, 205, 0], [205, 205, 0],
            [0, 0, 238], [205, 0, 205], [0, 205, 205], [229, 229, 229],
            [127, 127, 127], [255, 0, 0], [0, 255, 0], [255, 255, 0],
            [92, 92, 255], [255, 0, 255], [0, 255, 255], [255, 255, 255]
        ];
        let best = 0;
        let bestDist = Infinity;
        for (let i = 0; i < colors.length; i++) {
            const dr = r - colors[i][0];
            const dg = g - colors[i][1];
            const db = b - colors[i][2];
            const dist = dr * dr + dg * dg + db * db;
            if (dist < bestDist) {
                bestDist = dist;
                best = i;
            }
        }
        return best;
    }

    // ======================================================================
    // Erase operations
    // ======================================================================

    function eraseDisplay(mode) {
        switch (mode) {
            case 0: // Erase from cursor to end
                for (let x = cursorX; x < COLS; x++) {
                    grid[cursorY][x] = makeCell();
                }
                for (let y = cursorY + 1; y < ROWS; y++) {
                    for (let x = 0; x < COLS; x++) {
                        grid[y][x] = makeCell();
                    }
                }
                break;
            case 1: // Erase from start to cursor
                for (let y = 0; y < cursorY; y++) {
                    for (let x = 0; x < COLS; x++) {
                        grid[y][x] = makeCell();
                    }
                }
                for (let x = 0; x <= cursorX; x++) {
                    grid[cursorY][x] = makeCell();
                }
                break;
            case 2: // Erase entire display
                clearGrid();
                break;
        }
    }

    function eraseLine(mode) {
        switch (mode) {
            case 0: // Erase from cursor to end of line
                for (let x = cursorX; x < COLS; x++) {
                    grid[cursorY][x] = makeCell();
                }
                break;
            case 1: // Erase from start to cursor
                for (let x = 0; x <= cursorX; x++) {
                    grid[cursorY][x] = makeCell();
                }
                break;
            case 2: // Erase entire line
                for (let x = 0; x < COLS; x++) {
                    grid[cursorY][x] = makeCell();
                }
                break;
        }
    }

    function insertLines(count) {
        for (let i = 0; i < count; i++) {
            const newLine = [];
            for (let x = 0; x < COLS; x++) newLine.push(makeCell());
            grid.splice(cursorY, 0, newLine);
            grid.pop();
        }
    }

    function deleteLines(count) {
        for (let i = 0; i < count; i++) {
            grid.splice(cursorY, 1);
            const newLine = [];
            for (let x = 0; x < COLS; x++) newLine.push(makeCell());
            grid.push(newLine);
        }
    }

    function deleteChars(count) {
        for (let i = 0; i < count; i++) {
            grid[cursorY].splice(cursorX, 1);
            grid[cursorY].push(makeCell());
        }
    }

    function insertChars(count) {
        for (let i = 0; i < count; i++) {
            grid[cursorY].splice(cursorX, 0, makeCell());
            grid[cursorY].pop();
        }
    }

    function resetTerminal() {
        attr = { fg: 7, bg: 0, bold: false, underline: false, reverse: false };
        cursorX = 0;
        cursorY = 0;
        savedCursorX = 0;
        savedCursorY = 0;
        clearGrid();
    }

    // ======================================================================
    // Rendering
    // ======================================================================

    function requestRender() {
        if (renderPending) return;
        renderPending = true;
        requestAnimationFrame(render);
    }

    function showWelcomeBanner() {
        var banner = [
            '  ╔══════════════════════════════════╗',
            '  ║  ┌────────────────────────────┐  ║',
            '  ║  ╎                            ╎  ║',
            '  ║  ╎       octo-telnet          ╎  ║',
            '  ║  ╎   raw bytes, bare hands    ╎  ║',
            '  ║  ╎                            ╎  ║',
            '  ║  ╎   > _                      ╎  ║',
            '  ║  └────────────────────────────┘  ║',
            '  ╚══════════════════════════════════╝',
            '',
            '  Enter a BBS address above and click Connect.',
        ];
        clearGrid();
        cursorX = 0;
        cursorY = 0;
        for (var i = 0; i < banner.length; i++) {
            cursorX = 0;
            for (var j = 0; j < banner[i].length; j++) {
                putChar(banner[i][j], 1);
            }
            cursorY++;
        }
        requestRender();
    }

    function render() {
        renderPending = false;

        // Build HTML for the terminal output
        let html = '';
        for (let y = 0; y < ROWS; y++) {
            let lineHtml = '';
            let currentClass = null;
            let spanContent = '';

            for (let x = 0; x < COLS; x++) {
                const cell = grid[y][x];
                // Skip continuation cells from double-width characters
                if (cell.width === 0) continue;

                const cls = cellClass(cell);

                if (cls !== currentClass) {
                    if (spanContent) {
                        html += '<span class="' + (currentClass || '') + '">' + escapeHtml(spanContent) + '</span>';
                    }
                    currentClass = cls;
                    spanContent = '';
                }
                spanContent += cell.ch;
            }

            if (spanContent) {
                html += '<span class="' + (currentClass || '') + '">' + escapeHtml(spanContent) + '</span>';
            }

            if (y < ROWS - 1) {
                html += '\n';
            }
        }

        output.innerHTML = html;

        // Position the cursor
        if (cursorVisible && connected) {
            cursor.style.display = 'block';
            const charWidth = 9.5;  // Approximate character width in pixels
            const charHeight = 18;   // Line height in pixels
            const paddingLeft = 20;
            const paddingTop = 16;
            cursor.style.left = (paddingLeft + cursorX * charWidth) + 'px';
            cursor.style.top = (paddingTop + cursorY * charHeight) + 'px';
        } else {
            cursor.style.display = 'none';
        }
    }

    function cellClass(cell) {
        const classes = [];
        if (cell.reverse) {
            classes.push('ansi-reverse');
        } else {
            classes.push('ansi-fg-' + cell.fg);
            if (cell.bg !== 0) {
                classes.push('ansi-bg-' + cell.bg);
            }
        }
        if (cell.bold) classes.push('ansi-bold');
        if (cell.underline) classes.push('ansi-underline');
        return classes.join(' ');
    }

    function escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }

    // ======================================================================
    // WebSocket connection
    // ======================================================================

    function connect() {
        const host = hostInput.value.trim();
        if (!host) {
            alert('Please enter a BBS server address (e.g., bbs.example.com:23)');
            return;
        }

        setStatus('connecting');
        connectBtn.disabled = true;
        disconnectBtn.disabled = false;

        // Build WebSocket URL from the current page's host
        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = wsProtocol + '//' + window.location.host + '/';

        try {
            ws = new WebSocket(wsUrl);
            ws.binaryType = 'arraybuffer';
        } catch (e) {
            setStatus('disconnected');
            connectBtn.disabled = false;
            disconnectBtn.disabled = true;
            alert('Failed to create WebSocket: ' + e.message);
            return;
        }

        ws.onopen = function () {
            // Send the target Telnet address as the first message
            ws.send(host);
            setStatus('connected');
            connected = true;
            connectionInfo.textContent = 'Connected to ' + host;
            resetTerminal();
        };

        ws.onmessage = function (event) {
            const data = new Uint8Array(event.data);
            processData(data);
        };

        ws.onerror = function () {
            // Error is usually followed by close
        };

        ws.onclose = function () {
            setStatus('disconnected');
            connected = false;
            connectBtn.disabled = false;
            disconnectBtn.disabled = true;
            connectionInfo.textContent = '';
            ws = null;
            showWelcomeBanner();
        };
    }

    function disconnect() {
        if (ws) {
            ws.close();
            ws = null;
        }
        setStatus('disconnected');
        connected = false;
        connectBtn.disabled = false;
        disconnectBtn.disabled = true;
        connectionInfo.textContent = '';
        showWelcomeBanner();
    }

    function setStatus(status) {
        statusIndicator.className = 'status-' + status;
        statusIndicator.textContent = status.charAt(0).toUpperCase() + status.slice(1);
    }

    // ======================================================================
    // Keyboard input
    // ======================================================================

    document.addEventListener('keydown', function (e) {
        if (!connected || !ws) return;

        let sent = false;

        // Special key handling
        switch (e.key) {
            case 'Enter':
                sendBytes([0x0d, 0x0a]);
                sent = true;
                break;
            case 'Backspace':
                sendBytes([0x08]);
                sent = true;
                break;
            case 'Tab':
                sendBytes([0x09]);
                sent = true;
                break;
            case 'Escape':
                sendBytes([0x1b]);
                sent = true;
                break;
            case 'ArrowUp':
                sendBytes([0x1b, 0x5b, 0x41]); // ESC [ A
                sent = true;
                break;
            case 'ArrowDown':
                sendBytes([0x1b, 0x5b, 0x42]); // ESC [ B
                sent = true;
                break;
            case 'ArrowRight':
                sendBytes([0x1b, 0x5b, 0x43]); // ESC [ C
                sent = true;
                break;
            case 'ArrowLeft':
                sendBytes([0x1b, 0x5b, 0x44]); // ESC [ D
                sent = true;
                break;
            case 'Home':
                sendBytes([0x1b, 0x5b, 0x48]); // ESC [ H
                sent = true;
                break;
            case 'End':
                sendBytes([0x1b, 0x5b, 0x46]); // ESC [ F
                sent = true;
                break;
            case 'PageUp':
                sendBytes([0x1b, 0x5b, 0x35, 0x7e]); // ESC [ 5 ~
                sent = true;
                break;
            case 'PageDown':
                sendBytes([0x1b, 0x5b, 0x36, 0x7e]); // ESC [ 6 ~
                sent = true;
                break;
            case 'Insert':
                sendBytes([0x1b, 0x5b, 0x32, 0x7e]); // ESC [ 2 ~
                sent = true;
                break;
            case 'Delete':
                sendBytes([0x1b, 0x5b, 0x33, 0x7e]); // ESC [ 3 ~
                sent = true;
                break;
        }

        // Ctrl key combinations
        if (e.ctrlKey && e.key.length === 1) {
            const code = e.key.toLowerCase().charCodeAt(0) - 96;
            if (code >= 1 && code <= 26) {
                sendBytes([code]);
                sent = true;
            }
        }

        // Regular printable characters
        if (!sent && e.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) {
            sendBytes(Array.from(new TextEncoder().encode(e.key)));
            sent = true;
        }

        if (sent) {
            e.preventDefault();
        }
    });

    function sendBytes(bytes) {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(new Uint8Array(bytes));
        }
    }

    // ======================================================================
    // Event listeners
    // ======================================================================

    connectBtn.addEventListener('click', connect);
    disconnectBtn.addEventListener('click', disconnect);

    hostInput.addEventListener('keydown', function (e) {
        if (e.key === 'Enter') {
            connect();
        }
    });

    // Encoding selector
    document.getElementById('encoding-select').addEventListener('change', function (e) {
        if (connected) {
            disconnect();
        }
        setEncoding(e.target.value);
    });

    // Focus the terminal area on click
    document.getElementById('crt-screen').addEventListener('click', function () {
        // Redirect focus to host input if not connected
        if (!connected) {
            hostInput.focus();
        }
    });

    // Initial render
    showWelcomeBanner();
})();
