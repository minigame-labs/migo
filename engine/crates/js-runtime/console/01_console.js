import { op_console } from "ext:core/ops";

class Console {
    log(...args) {
        op_console(args, 1);
    }
    warn(...args) {
        op_console(args, 2);
    }
    error(...args) {
        op_console(args, 3);
    }
    info(...args) {
        op_console(args, 1);
    }
    debug(...args) {
        op_console(args, 1);
    }
}

const console = new Console();
export { console };