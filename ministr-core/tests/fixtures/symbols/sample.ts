/** Maximum retry count. */
export const MAX_RETRIES = 3;

/** A trait-like interface for serialization. */
export interface Serializable {
    serialize(): string;
}

/** Configuration for the application. */
export class AppConfig implements Serializable {
    constructor(
        public name: string,
        public debug: boolean = false,
    ) {}

    isDebug(): boolean {
        return this.debug;
    }

    serialize(): string {
        return JSON.stringify(this);
    }
}

/** Greet a user by name. */
export function greet(name: string): string {
    return `Hello, ${name}!`;
}
