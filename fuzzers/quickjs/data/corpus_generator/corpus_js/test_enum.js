function fuzz() {
var a, tab;
a = {x:1,
     "18014398509481984": 1,
     "9007199254740992": 1,
     "9007199254740991": 1,
     "4294967296": 1,
     "4294967295": 1,
     y:1,
     "4294967294": 1,
     "1": 2};
tab = Object.keys(a);
}
fuzz();
