import { EventTarget } from "./02_event.js";

class Element extends EventTarget {
    constructor(tagName = '') {
        super();
        this.tagName = tagName;
        this.id = '';
        this.className = '';
        this.style = {};
    }

    setAttribute(name, value) {
        this[name] = value;
    }

    getAttribute(name) {
        return this[name];
    }
}

class HTMLElement extends Element {
    constructor(tagName = 'HTML') {
        super(tagName);
    }
}

export {
    HTMLElement,
};