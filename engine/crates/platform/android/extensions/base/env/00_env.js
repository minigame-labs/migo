import { op_get_user_data_path } from "ext:core/ops";

const env = {
    USER_DATA_PATH: op_get_user_data_path()
}

export { env };