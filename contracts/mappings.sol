// Mapping slots must match solc: keccak256(pad32(key) ++ slot), and for a
// string key keccak256(key ++ slot).
contract mappings {
	struct Point {
		uint64 x;
		int64 y;
	}

	uint256 public total;
	mapping(uint256 => uint256) public byNumber;
	mapping(address => uint256) public byAddress;
	mapping(address => mapping(address => uint256)) public nested;
	mapping(bytes4 => bool) public flags;
	mapping(int8 => uint8) public bySigned;
	mapping(uint256 => Point) public points;
	mapping(string => uint256) public byName;

	function setNumber(uint256 key, uint256 value) public {
		byNumber[key] = value;
		total += value;
	}

	function setAddress(address key, uint256 value) public {
		byAddress[key] = value;
	}

	function setNested(address a, address b, uint256 value) public {
		nested[a][b] = value;
	}

	function addNested(address a, address b, uint256 value) public {
		nested[a][b] += value;
	}

	function setFlag(bytes4 key, bool value) public {
		flags[key] = value;
	}

	function setSigned(int8 key, uint8 value) public {
		bySigned[key] = value;
	}

	function setPoint(uint256 key, uint64 x, int64 y) public {
		points[key] = Point(x, y);
	}

	function clearPoint(uint256 key) public {
		delete points[key];
	}

	function setAlice(uint256 value) public {
		byName["alice"] = value;
	}

	function alice() public view returns (uint256) {
		return byName["alice"];
	}

	function hashWord(uint256 a) public pure returns (bytes32) {
		return keccak256(abi.encode(a));
	}

	function hashEmpty() public pure returns (bytes32) {
		return keccak256("");
	}

	function hashText() public pure returns (bytes32) {
		return keccak256("hello");
	}
}
