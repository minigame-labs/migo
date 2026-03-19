import { getFileSystemManager } from "./02_file_manager.js";

const saveFileToDisk = () => {
    throw new Error("Not Supported");
};

function saveFile(options = {}) {
    return getFileSystemManager().saveFile(options);
}

function saveFileSync(options = {}) {
    return getFileSystemManager().saveFileSync(options);
}

export { saveFileToDisk, getFileSystemManager, saveFile, saveFileSync };
