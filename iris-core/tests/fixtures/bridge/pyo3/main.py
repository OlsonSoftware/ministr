# PyO3 imports — Python side
from _mymodule import hello, Config

msg = hello("World")
cfg = Config()
debug = cfg.is_debug()
