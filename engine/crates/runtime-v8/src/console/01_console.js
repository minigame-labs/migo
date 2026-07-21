import { op_console } from "ext:core/ops";

let groupDepth = 0;

function formatArgs(args) {
    const indent = "  ".repeat(groupDepth);
    const msg = args.map((a) => {
        if (typeof a === "string") return a;
        try { return JSON.stringify(a); } catch (_) { return String(a); }
    }).join(" ");
    return indent + msg;
}

class Console {
    debug(...args) {
        op_console(formatArgs(args), 0);
    }
    log(...args) {
        op_console(formatArgs(args), 1);
    }
    info(...args) {
        op_console(formatArgs(args), 1);
    }
    warn(...args) {
        op_console(formatArgs(args), 2);
    }
    error(...args) {
        op_console(formatArgs(args), 3);
    }
    group(label) {
        op_console("  ".repeat(groupDepth) + (label ?? ""), 1);
        groupDepth++;
    }
    groupEnd() {
        if (groupDepth > 0) groupDepth--;
    }
}

const console = new Console();
export { console };
