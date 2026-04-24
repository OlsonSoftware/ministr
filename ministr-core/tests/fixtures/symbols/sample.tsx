/** Props for the Greeting component. */
export interface GreetingProps {
    name: string;
    className?: string;
}

/** A greeting component. */
export function Greeting({ name, className }: GreetingProps) {
    return <h1 className={className}>Hello, {name}!</h1>;
}

/** Application configuration. */
export class AppConfig {
    constructor(public name: string) {}
}

/** Default greeting message. */
export const DEFAULT_MESSAGE = "Hello, World!";
