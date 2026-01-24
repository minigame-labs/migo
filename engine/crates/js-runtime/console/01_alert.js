import { op_console } from "ext:core/ops";

const alert = (...args) => {
    op_console(args, 1);
}

export { alert };