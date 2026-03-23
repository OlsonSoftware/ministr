// NAPI-RS imports — JavaScript side
import { add, getVersion, Calculator } from './native';

const sum = add(1, 2);
const ver = getVersion();
const calc = new Calculator();
