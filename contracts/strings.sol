// Dynamic ABI types: string and bytes arguments and returns, and
// require(cond, "message") reverting with Error(string).
contract strings {
	string public constant name = "Prova Token";

	function echo(string memory s) public pure returns (string memory) {
		return s;
	}

	function echoBytes(bytes memory b) public pure returns (bytes memory) {
		return b;
	}

	// Static and dynamic values mixed: offsets point past the head.
	function mixed(uint8 a, string memory s, uint256 b, bytes memory c)
		public
		pure
		returns (uint256, string memory, uint8, bytes memory)
	{
		return (b, s, a, c);
	}

	function length(string memory s) public pure returns (uint256) {
		return bytes(s).length;
	}

	function hash(string memory s) public pure returns (bytes32) {
		return keccak256(bytes(s));
	}

	function check(uint256 x) public pure returns (uint256) {
		require(x < 10, "too big");
		return x;
	}

	function long() public pure returns (string memory) {
		return "a string longer than thirty-two bytes, so it needs two words";
	}
}
