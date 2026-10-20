// Division and modulo edge cases, narrow (one register) and wide (helpers).
contract arith {
	function udiv8(uint8 a, uint8 b) public pure returns (uint8) { return a / b; }
	function umod64(uint64 a, uint64 b) public pure returns (uint64) { return a % b; }
	function sdiv32(int32 a, int32 b) public pure returns (int32) { return a / b; }
	function smod16(int16 a, int16 b) public pure returns (int16) { return a % b; }
	function udiv256(uint256 a, uint256 b) public pure returns (uint256) { return a / b; }
	function sdiv128(int128 a, int128 b) public pure returns (int128) { return a / b; }
	function smod128(int128 a, int128 b) public pure returns (int128) { return a % b; }
}
