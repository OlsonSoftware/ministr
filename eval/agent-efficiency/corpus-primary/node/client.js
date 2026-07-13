const { encodePacket } = require("./native.node");

module.exports = (value) => encodePacket(value);
